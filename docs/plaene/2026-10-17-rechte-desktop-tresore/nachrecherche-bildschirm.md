# S4 — Nachrecherche N1–N25 und Revalidierung Abschnitt C

Stand und Abrufdatum 2026-10-17. Alle Web-Quellen am 2026-10-17 abgerufen. Lesestand des Repositorys: `da2646d2`.

Bezug: `strang-bildschirm.md` (Abschnitt C „Entscheidungen“, Abschnitt G „Nachrecherche“). Diese Datei bewertet nicht neu und entscheidet nicht. Kennzeichnung: **Folgerung:** = eigener Schluss für den Plan; **Annahme:** = ungeprüft; **Lücke:** = genau benannte fehlende Angabe.

Quellenarten und Schreibweise:
- `N/…` = `native/src/…` im Repository; `V/…` = `vendor/…` im Repository.
- Crate-Quelltexte im lokalen Cargo-Register (`~/.cargo/registry/src/index.crates.io-*/`, nur gelesen): `egui-0.29.1`, `egui-winit-0.29.1`, `egui-wgpu-0.29.1`, `eframe-0.29.1`, `wgpu-22.1.0`, `wgpu-types-22.0.0`, `wgpu-hal-22.0.0`, `windows-0.58.0`, `x11rb-0.13.2`, `rustls-0.23.41` (Lock: 0.23.46; die ALPN-Stelle ist in 0.23.x unverändert — Annahme, 0.23.46 nicht im Register).
- Crate-Quelltexte, die nicht im Register liegen, als Archiv von `https://static.crates.io/crates/<name>/<name>-<version>.crate` bzw. `https://crates.io/api/v1/crates/openh264-sys2/0.9.9/download` geladen und nur gelesen: `iroh-1.3.0`, `noq-1.3.0`, `noq-proto-1.3.0`, `iroh-relay-1.3.0`, `ashpd-0.13.13`, `openh264-0.9.9`, `openh264-sys2-0.9.9`, `windows-capture-2.0.1`. Zeilenangaben beziehen sich auf diese Archive.
- Keine Builds, keine Tests, keine Ausführung von Programmen aus den Archiven. Die zwei Cisco-Binärdateien wurden nur heruntergeladen, entpackt und gehasht (nicht ausgeführt).

---

## 1. Antworten N1–N25

### N1 — iroh 1.3.0: Uni-Streams je Verbindung, Staukontrolle, Datagrammpuffer, Kredit nach `reset`

**Antwort.**
1. `Connection::set_max_concurrent_uni_streams(&self, count: VarInt)`, `open_uni()` und `accept_uni()` existieren in iroh 1.3.0 je Verbindung, unverändert gegenüber 1.0.1. Dokumentation: „No streams may be opened by the peer unless fewer than `count` are already open.“ Der Wert begrenzt also die Streams, die die **Gegenseite** öffnen darf; der Betrachter ruft ihn auf seiner Verbindung auf, damit der Host Uni-Streams öffnen kann.
2. Staukontrolle: iroh 1.3.0 überschreibt den `noq`-Standard nicht. `QuicTransportConfigBuilder::new()` setzt nur Keepalive, Pfad-Leerlauf, Multipath-Pfadzahl, NAT-Adressen und Handshake-Migration; Standard bleibt `congestion::CubicConfig::default()` aus `noq-proto` 1.3.0 (Tag-Version, nicht Zweig `main`).
3. Datagrammpuffer in `noq-proto` 1.3.0: Empfang `Some(STREAM_RWND)` = 12 500 000 B/s ÷ 1000 × 100 ms = 1 250 000 Byte; Senden 1 048 576 Byte. iroh überschreibt beide nicht. Das Projekt setzt keine Datagramm-Einstellung (`N/share/core/keepalive.rs:101-118`).
4. `SendStream::reset`: Lokal gepufferte Daten werden sofort verworfen, nichts wird mehr wiederholt („Locally buffered data is dropped, and previously transmitted data will no longer be retransmitted if lost.“). Beim Empfänger rechnet `received_reset` die Endgröße (`final_offset`) an und gibt den noch nicht gelesenen Rest sofort als **Verbindungs**-Kredit frei (`add_read_credits(final_offset − bytes_read)`, `MAX_DATA` wird gesendet). Der Stream-Zählplatz (Uni-Stream-Limit) wird erst frei, wenn der Empfänger den Stream gestoppt hat oder die Anwendung den Reset gelesen und den Stream freigegeben hat (`stopped` → sofort `stream_recv_freed`, sonst nach Lesen).

**Belege.**
- `iroh-1.3.0/src/endpoint/connection.rs:885` (`open_uni`), `:904` (`accept_uni`), `:1124-1131` (`set_max_concurrent_uni_streams` mit Doku), `:1148` (`set_max_concurrent_bi_streams`), `:1139` (`set_receive_window`), `:1056` (`rtt(path_id)`), `:1062` (`stats()`).
- `iroh-1.3.0/src/endpoint/quic.rs:152-164` (Builder-Standardwerte ohne Staukontroll- und Datagramm-Überschreibung), `:392-424` (`datagram_receive_buffer_size`, `datagram_send_buffer_size`, `congestion_controller_factory`).
- `noq-proto-1.3.0/src/config/transport.rs:550-554` (`STREAM_RWND`), `:558` (`max_concurrent_uni_streams: 100`), `:581-582` (Datagrammpuffer), `:586` (`CubicConfig::default()`).
- `noq-1.3.0/src/send_stream.rs:204-213` (Doku `reset`).
- `noq-proto-1.3.0/src/connection/streams/state.rs:296-357` (`received_reset`, Verbindungskredit), `noq-proto-1.3.0/src/connection/streams/recv.rs:168-200` (Puffer leeren).

**Folgerung für den Plan:** C3 L1 trägt ohne endpunktweite Änderung; R2 entfällt als Risiko. Ein Reset gibt Verbindungskredit beim Empfänger sofort frei; der Betrachter muss einen zurückgesetzten Uni-Stream aktiv schließen (`stop`/Drop nach Lesen des Reset-Fehlers), sonst bleibt der Zählplatz belegt und das Limit `set_max_concurrent_uni_streams` wird mit der Zeit erschöpft.

**Lücke:** Wie schnell der **Sender** seinen Verbindungs-Sendekredit zurückerhält, hängt vom Eintreffen des `MAX_DATA`-Rahmens ab (eine Umlaufzeit); keine Messung.

### N2 — Erkennung einer fehlgeschlagenen ALPN-Aushandlung

**Antwort.** Eine ältere Gegenstelle ohne `smart-explorer/share-screen/1` in ihrer Endpoint-ALPN-Liste lehnt im TLS-Handshake ab: rustls (Server) sendet den Alarm `no_application_protocol` (120) mit `Error::NoApplicationProtocol`; QUIC bildet TLS-Alarme auf den Transportfehlercode `0x100 | alert` = `0x178` ab. Beim Verbindenden ergibt das in iroh 1.3.0 `ConnectError::Connecting { source: ConnectingError::ConnectionError { source: ConnectionError::ConnectionClosed(ConnectionClose { error_code, .. }) } }` mit `u64::from(error_code) == 0x178`. iroh baut die Server-TLS-Konfiguration aus der Endpoint-ALPN-Liste (`Builder::alpns`).

**Belege.**
- `rustls-0.23.41/src/server/hs.rs:98-136` (ALPN-Abgleich; für QUIC Pflichtabbruch mit `AlertDescription::NoApplicationProtocol`, Kommentar zitiert RFC 9001 „connection error of type 0x0178“).
- `noq-proto-1.3.0/src/transport_error.rs:69-74` (`Code::crypto(code) = 0x100 | code`), `:88` (`impl From<Code> for u64`); `noq-proto-1.3.0/src/crypto/rustls.rs:100-102` (Alarm → `TransportErrorCode::crypto`).
- `noq-proto-1.3.0/src/connection/mod.rs:7437-7468` (`ConnectionError::ConnectionClosed(frame::ConnectionClose)`); `noq-proto-1.3.0/src/frame.rs:900-908` (`ConnectionClose { error_code, frame_type, reason }`).
- `iroh-1.3.0/src/endpoint.rs:944-953` (`ConnectError`), `iroh-1.3.0/src/endpoint/connection.rs:467-482` (`ConnectingError`), `iroh-1.3.0/src/endpoint.rs:255` und `:546-547` (Server-Konfiguration aus `alpns`); Re-Export `TransportErrorCode` in `iroh-1.3.0/src/endpoint.rs:114`.
- Projekt: keine bestehende Auswertung von `0x178` (Suche `0x178|NoApplicationProtocol|ConnectionError::ConnectionClosed` in `N/` ohne Treffer).

**Folgerung für den Plan:** `screen_unsupported` = genau dieser Fall (`ConnectionClosed` mit Code `0x178` während des Verbindungsaufbaus). Andere Fehler (Zeitüberschreitung, `ApplicationClosed`) bleiben gewöhnliche Verbindungsfehler.

**Lücke:** Verhalten, wenn ein älterer Host über das Relay erreicht wird, ist nicht getrennt geprüft; der Abbruch geschieht im TLS-Handshake Ende-zu-Ende und ist pfadunabhängig (Annahme).

### N3 — eframe 0.29.1: Paint-Callback, `wgpu_render_state`, `present_mode`

**Antwort.**
1. `egui_wgpu::CallbackTrait` mit `prepare(device, queue, screen_descriptor, egui_encoder, callback_resources)`, `finish_prepare(…)` und `paint(info, render_pass: &mut wgpu::RenderPass<'static>, callback_resources)`; Beispielverweis im Doc-Kommentar auf `custom3d_wgpu`.
2. `eframe::CreationContext::wgpu_render_state: Option<egui_wgpu::RenderState>` und `Frame::wgpu_render_state()`.
3. `egui_wgpu::WgpuConfiguration { present_mode: wgpu::PresentMode, desired_maximum_frame_latency: Option<u32>, … }`, Standard `PresentMode::AutoVsync` und `desired_maximum_frame_latency: None` (dann gilt der wgpu-Standard 2). eframe übernimmt beide beim Konfigurieren der Fläche.
4. Mailbox mit Rückfall: wgpu 22 kennt keinen Modus „Mailbox, sonst Fifo“. `PresentMode::Mailbox` „will crash if unsupported“; `AutoNoVsync` wählt Immediate → Mailbox → Fifo; `AutoVsync` wählt FifoRelaxed → Fifo. Fifo: Warteschlange „approximately 3 frames long“.
5. `desired_maximum_frame_latency`: „Choose 1 for low latency from frame recording to frame display.“ Mit Warnung: ohne Present-Wait im Backend wartet die CPU in `get_current_texture` auf die GPU.

**Belege.** `egui-wgpu-0.29.1/src/renderer.rs:88-120`; `eframe-0.29.1/src/epi.rs:83`, `:693-694`; `egui-wgpu-0.29.1/src/lib.rs:224-249`, `:314-316`; `egui-wgpu-0.29.1/src/winit.rs:164-178`; `wgpu-types-22.0.0/src/lib.rs:5252-5260` (AutoVsync/AutoNoVsync), `:5316` (Mailbox), `:5432-5436` (Absturz bei nicht unterstütztem Mailbox), `:5445-5457` (`desired_maximum_frame_latency`); `wgpu-22.1.0/src/lib.rs:5643` (Standard 2).

**Folgerung für den Plan:** R15-Annahme „höchstens eine Bildwiederholperiode“ ist mit den Standardwerten nicht gedeckt (siehe Abschnitt 2). Niedriglatenz-Weg ohne Absturzrisiko: `present_mode = AutoNoVsync` (Rückfall eingebaut, erlaubt Tearing bei Immediate) oder `AutoVsync` mit `desired_maximum_frame_latency = Some(1)`. Ein „Mailbox, falls vorhanden“ verlangt eine Fähigkeitsabfrage der Fläche vor dem Start des eframe-Fensters, die eframe 0.29.1 nicht anbietet.

**Lücke:** Gemessene Zusatzlatenz je Modus auf Windows (DX12/Vulkan) und Linux (Vulkan/GL) fehlt.

### N4 — egui 0.29.1: fehlende Tasten und IME

**Antwort.** `egui::Key` enthält genau: Pfeiltasten (4), `Escape`, `Tab`, `Backspace`, `Enter`, `Space`, `Insert`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, `Copy`, `Cut`, `Paste`, die Satzzeichen `Colon`, `Comma`, `Backslash`, `Slash`, `Pipe`, `Questionmark`, `OpenBracket`, `CloseBracket`, `Backtick`, `Minus`, `Period`, `Plus`, `Equals`, `Semicolon`, `Quote`, `Num0`–`Num9`, `A`–`Z`, `F1`–`F35`.

Es **fehlen** als Tastenereignis: Super/Windows-Taste (links, rechts), Menü-/Kontexttaste, Druck (PrintScreen), Rollen (ScrollLock), Pause, Feststelltaste (CapsLock), NumLock, die Modifikatortasten Umschalt/Strg/Alt als eigene Ereignisse (nur als `Modifiers { alt, ctrl, shift, mac_cmd, command }`, kein Super-Bit), Medientasten (Wiedergabe, Lautstärke, Stumm), Browser-/Starttasten, ISO-Zusatztaste `IntlBackslash` und die Nummernblock-Unterscheidung (egui-winit bildet `Numpad0` auf `Num0`, `NumpadEnter` auf `Enter` ab). F13–F24 sind **vorhanden** (F1–F35).

IME: `Event::Ime(ImeEvent::{Enabled, Preedit(String), Commit(String), Disabled})`. egui-winit erlaubt IME am Fenster nur, solange `PlatformOutput::ime` gesetzt ist („set if, and only if, the user is currently editing text“).

**Belege.** `egui-0.29.1/src/data/key.rs:7` ff. (`pub enum Key`); `egui-0.29.1/src/data/input.rs:396` (`Event::Key`), `:475` (`Event::Ime`), `:541-553` (`ImeEvent`), `:588` ff. (`Modifiers`); `egui-0.29.1/src/data/output.rs:116-119` (`ime`); `egui-winit-0.29.1/src/lib.rs:844-845` (`set_ime_allowed` aus `ime.is_some()`), `:1127-1169` (`key_from_key_code`, Nummernblock-Zusammenlegung).

**Folgerung für den Plan:** R9 tritt ein. Der Betrachter muss für Super, Menü, Druck, Pause, Rollen, CapsLock, NumLock und Medientasten Werkzeugleisten-Knöpfe anbieten. Für Text in Ostasiatischen Sprachen muss der Betrachter `ctx.output_mut(|o| o.ime = Some(IMEOutput { … }))` setzen, solange die Fernsteuerung Tastatureingaben aufnimmt; die übertragene Einheit ist dann `Text{utf8}` aus `Commit`.

**Lücke:** Ob eframe 0.29.1 Rohereignisse von winit an die Anwendung durchreicht (für die Super-Taste), wurde nicht gefunden; Annahme: nein.

### N5 — `windows` 0.58: Features, Rahmenpool, sicherer Desktop

**Antwort.**
- Feature-Namen in `windows-0.58.0/Cargo.toml` vorhanden: `Graphics_Capture` (Zeile 185, setzt `Graphics` voraus), `Graphics_DirectX`, `Graphics_DirectX_Direct3D11`, `Win32_System_WinRT_Graphics_Capture` (Zeile 693), `Win32_System_WinRT_Direct3D11` (Zeile 690), `Win32_Graphics_Direct3D11`, `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`, `Win32_Graphics_Dwm`, `Win32_Media_MediaFoundation` (Zeile 491), `Win32_UI_Input_KeyboardAndMouse`, `Win32_System_StationsAndDesktops`, `Win32_Security`, `Win32_System_Threading`, `Win32_System_Com_StructuredStorage`, `Win32_System_Variant`, `Security_Authorization_AppCapabilityAccess`, `Win32_Security_Authentication_Identity` (für `SendSAS`, `…/Identity/mod.rs:1202`).
- `Direct3D11CaptureFramePool::CreateFreeThreaded(device, pixelformat, numberofbuffers, size)` vorhanden (`windows-0.58.0/src/Windows/Graphics/Capture/mod.rs:326`); freilaufender Pool braucht keine `DispatcherQueue` (Folgerung aus dem Namen und der Microsoft-Beschreibung „free-threaded“; Microsoft-Zitat nicht abgerufen — Lücke).
- Weitere in 0.58 vorhandene WGC-Elemente: `GraphicsCaptureAccess::RequestAccessAsync` (`:361`), `GraphicsCaptureItem::TryCreateFromWindowId` (`:420`), `TryCreateFromDisplayId` (`:426`), `SetIsCursorCaptureEnabled` (`:508`), `IsBorderRequired`/`SetIsBorderRequired` (`:512-519`), `SetMinUpdateInterval` (`:541`), `SetIncludeSecondaryWindows` (`:552`), `GraphicsCaptureSession::IsSupported` (`:556`).
- `OpenInputDesktop` (`…/StationsAndDesktops/mod.rs:210`) und `GetUserObjectInformationW` (`:182`) vorhanden.
- WGC beim Wechsel auf den sicheren Desktop: **Lücke** — weder die WGC-Doku noch `OpenInputDesktop` beschreiben das Verhalten. Microsoft-Doku `OpenInputDesktop`: liefert „the desktop that receives user input“, bei Fehler `NULL` + `GetLastError`; der Winlogon-Desktop wird nicht erwähnt.

**Belege.** Dateien oben; https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openinputdesktop (updated_at 2025-07-01).

**Folgerung für den Plan:** Erkennung „sicherer Desktop“ muss empirisch festgelegt werden: `OpenInputDesktop(0, false, DESKTOP_READOBJECTS)` scheitert oder `GetUserObjectInformationW(UOI_NAME)` ≠ `Default` → `secure_desktop`. Annahme bis zur Messung auf einem Windows-Runner.

### N6 — Media Foundation: Hardware-MFTs, Server/N-Editionen, Patentlage

**Antwort.**
- Profil „Constrained Baseline“ ist in MF als `eAVEncH264VProfile_ConstrainedBase = 256` („H.264 extension“) definiert, neben `eAVEncH264VProfile_Base = 66`; Wert gilt für `MF_MT_MPEG2_PROFILE`.
- Pflichtattribute für zertifizierte Hardware-Encoder (aus Ref B1, Microsoft-Seite „H.264 Video Encoder“): `CODECAPI_AVEncVideoForceKeyFrame` und `CODECAPI_AVEncCommonMeanBitRate` gehören zur Pflichtliste. Folgerung: Jeder HCK-zertifizierte Hardware-MFT (NVIDIA, Intel, AMD) muss Schlüsselbildanforderung und Bitrate annehmen; ob die **Änderung zur Laufzeit** bei jedem Hersteller-MFT wirkt, ist nicht belegt.
- Ob NVIDIA-, Intel- und AMD-MFTs `ConstrainedBase` (256) annehmen: **Lücke** — keine Herstellerdokumentation gefunden.
- Windows N: Microsoft nennt „media-related technologies, Windows Media Player, and certain preinstalled media apps“ als ausgeschlossen; Media Foundation ist nicht namentlich genannt. Nachrüstung: Einstellungen → Apps → Optionale Features → „Media Feature Pack“ (ab Windows 10 1909 nur als optionales Feature).
- Windows Server: Feature `Server-Media-Foundation`; ob seine Installation einen Neustart verlangt, ist von Microsoft nicht als Primärquelle gefunden; Forenbeiträge melden Neustartbedarf (Docker-Forum, siehe N7).
- MF-H.264-**Decoder**: „supports decoding of Baseline, Main, and High profiles, up to level 5.1“, höchstens 4096 × 2304, DXVA 2 garantiert bis 1920 × 1088, DXVA nur für „Main-compatible Baseline“ (profile_idc 66 mit constraint_set1_flag = 1), Main und High; Ausgabe u. a. `MFVideoFormat_NV12`.
- Patentlage der OS-Codecs: **Lücke** — keine Primärquelle von Microsoft oder Google zur AVC-Patentlizenz der eingebauten Codecs gefunden.

**Belege.** https://learn.microsoft.com/en-us/windows/win32/api/codecapi/ne-codecapi-eavench264vprofile (updated_at 2024-02-22); https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-decoder; https://support.microsoft.com/en-us/topic/media-feature-pack-list-for-windows-n-editions-c1c6fffa-d052-8338-7a79-a4bb980a700a; Ref B1.

**Folgerung für den Plan:** Der Host muss für `cb` `eAVEncH264VProfile_ConstrainedBase` setzen und bei Ablehnung durch einen Hardware-MFT auf den Software-MFT bzw. auf `high` (siehe N8/Abschnitt 2 zu OpenH264-Dekodierung) ausweichen. Für Windows-Betrachter mit Hardware-Dekodierung ist ein Baseline-Strom nur dann DXVA-fähig, wenn constraint_set1_flag = 1 gesetzt ist (Constrained Baseline erfüllt das).

### N7 — GitHub-Windows-Runner: Sitzung, Media Foundation, erhöhtes Fenster

**Antwort.**
- Interaktive Sitzung: Ein Mitarbeiter von `actions/runner-images` hat am 2026-08-07 bestätigt: „the session is actually interactive on `windows-11-arm` — `UserInteractive=True`, it's on the interactive window station (`WinSta0`)“; auf dem ARM-Bild scheitert nur `SetForegroundWindow`, das Vergleichsbild (x64) nicht. Folgerung: WGC, `SendInput` und UI Automation laufen auf x64-Runnern in einer interaktiven Sitzung; nicht gemessen.
- Bildstände: `windows-2022` = OS 10.0.20348 Build 5622, Image 20261012.333.1; `windows-2025` = OS 10.0.26100 Build 33438, Image 20261013.288.2.
- Media Foundation: Die installierten Windows-Features der Toolsets 2022 und 2025 sind genau `Containers`, `Microsoft-Windows-Subsystem-Linux`, `VirtualMachinePlatform`, `NET-Framework-45-Features`, `Client-ProjFS`, `NET-Framework-Features`, `Hyper-V`, `HypervisorPlatform`, `Hyper-V-PowerShell`, `Wireless-Networking`; `Server-Media-Foundation` ist nicht dabei. Neustartbedarf der Nachinstallation: nur Forenbelege (Docker-Forum: Installation verlangt Neustart, im Container nicht möglich). Ein Neustart innerhalb eines GitHub-Jobs ist nicht vorgesehen (Annahme).
- Erhöhtes Testfenster: **Lücke** — keine Quelle geprüft. Annahme: Der Runner-Benutzer `runneradmin` ist Administrator; ob UAC aktiv ist (dann Medium- und High-IL-Prozesse nebeneinander), ist nicht belegt.

**Belege.** https://github.com/actions/runner-images/issues/14049 (Kommentar v-davit-ioramashvili, 2026-08-07); https://raw.githubusercontent.com/actions/runner-images/main/images/windows/Windows2022-Readme.md und `…/Windows2025-Readme.md`; `…/images/windows/toolsets/toolset-2022.json` und `toolset-2025.json` (`windowsFeatures`); https://forums.docker.com/t/install-media-foundation-on-windows-ltsc-2022/127741.

**Folgerung für den Plan:** R6 tritt für Media Foundation ein: MF-Kodierung ist auf GitHub-Windows-Runnern ohne Neustart voraussichtlich nicht prüfbar; Windows-Kodierung in CI über OpenH264 (Cisco-Datei) oder `tiles`, MF-Pfad als ungeprüft berichten. Aufnahme (WGC/DXGI) und `SendInput` sind prüfbar.

### N8 — OpenH264 (Cisco): Version, URLs, HTTPS, Hashes, Signatur, Lizenz, Crate, `.bz2`

**Antwort.**
- Aktuelle Version: GitHub-Release „latest“ = `v2.6.0` (2025-02-12); daneben `2.5.1` (2025-03-12) als Sicherheitsnachtrag („Fix decoder heap overflow vulnerability (Commit: 63db555e, PR: #3818)“). Commit `63db555e` (2024-12-19) ist in `v2.6.0` enthalten (GitHub-Compare `v2.6.0...63db555e`: „behind_by 7“).
- Datei-URLs 2.6.0 (Release-Text nennt `http://`): `ciscobinary.openh264.org/openh264-2.6.0-win64.dll.bz2`, `…-win32.dll.bz2`, `…-win-arm64.dll.bz2`, `libopenh264-2.6.0-linux64.8.so.bz2`, `…-linux32.8.so.bz2`, `…-linux-arm64.8.so.bz2`, `…-linux-arm.8.so.bz2`, `…-android-arm64.8.so.bz2`, `…-android-arm.8.so.bz2`, `…-android-x64.8.so.bz2`, `…-android-x86.8.so.bz2`, `…-mac-x64.dylib.bz2`, `…-mac-arm64.dylib.bz2`, `…-ios.a.bz2`, je mit `.signed.md5.txt`.
- HTTPS: `https://ciscobinary.openh264.org/openh264-2.6.0-win64.dll.bz2` antwortet `HTTP/2 200`, `server: AmazonS3`, `content-length: 452053`, `last-modified: Tue, 01 Sep 2026 07:59:24 GMT`; Linux-Datei `last-modified: Tue, 01 Sep 2026 07:59:22 GMT`, `content-length: 634264`.
- SHA-256 (am 2026-10-17 selbst berechnet):
  - `openh264-2.6.0-win64.dll.bz2` = `dab5f2a872777f9a58b69bfa9fbcf20d9f82f2d6ec91383fd70bff49bd34ac9f`; entpackt `openh264-2.6.0-win64.dll` = `2076cb5675ec6c1a4c70e7a2a322552f547b6eeed649d6dfcd9e02a543b24691`.
  - `libopenh264-2.6.0-linux64.8.so.bz2` = `27ab53323c110b76214c1c72222f459d17febbcd1e252136cadc292b0308d75b`; entpackt `libopenh264-2.6.0-linux64.8.so` = `2f0cde7c6a6abcf5cae76942894ea42897fa677bce4ed6c91a24dd1b041d5f04`.
  - Beide entpackten Werte stimmen mit der Hash-Liste im Crate `openh264-sys2` 0.9.9 überein (`src/blobs/hashes.txt:16` und `:22`).
- Signaturverfahren: Die `.signed.md5.txt`-Dateien enthalten nur eine MD5-Zeile (Windows: `ce1282f5845f56761954282e4992730d *openh264-2.6.0-win64.dll`, 59 Byte; Linux: `MD5 (libopenh264-2.6.0-linux64.8.so) = 1859c0aaf825429cbf36f1f496c5e08c`), keine kryptografische Signatur. Die Windows-DLL trägt eine Authenticode-Signatur (PE-Sicherheitsverzeichnis vorhanden: Offset 957440, 21080 Byte; Gültigkeit nicht geprüft). Die Linux-`.so` hat keinen Signaturabschnitt (ELF-Abschnitte ohne Signatur, nur `.note.gnu.build-id`). Release-Text: „All the binaries have been digitally signed.“
- Binärlizenz (vollständig, v1.0): BSD-2-Clause-Text für Cisco 2014 plus AVC-Patent-Hinweis „FOR THE PERSONAL USE OF A CONSUMER OR OTHER USES IN WHICH IT DOES NOT RECEIVE REMUNERATION …“ plus vier Bedingungen: (1) „separately downloaded to an end user's device, and not integrated into or combined with third party software prior to being downloaded“; (2) Nutzer kann die Nutzung ein-, aus- und wieder einschalten; (3) an der Stelle der Steuerung Anzeige „OpenH264 Video Codec provided by Cisco Systems, Inc.“; (4) „Any third-party software that makes use of the Cisco-provided binary must reproduce all of the above text, as well as this last condition, in the EULA and/or in another location where licensing information is to be presented to the end user.“
- Crate `openh264` 0.9.9 (BSD-2-Clause, 2026-10-15): Features `default = ["source"]`, `source`, `libloading` (→ `openh264-sys2/libloading` mit `libloading` ^0.8.1 und `sha2` ^0.10.8; Lock hat `libloading` 0.8.9 und `sha2` 0.10.9). Mit `default-features = false, features = ["libloading"]` wird kein C-Code gebaut. API: `OpenH264API::from_blob_path(path)` prüft den SHA-256 gegen `src/blobs/hashes.txt` (23 Einträge, nur Version 2.6.0) und lädt dann; `from_blob_path_unchecked` (unsafe) lädt ohne Prüfung. Zusätzliche Pflichtabhängigkeit des Crates: `wide` (nicht im Lock).
- `.bz2`-Entpacker: `bzip2` 0.6.1 (MIT OR Apache-2.0, 2025-10-16) nutzt standardmäßig `libbz2-rs-sys` (reines Rust, Lizenz `bzip2-1.0.6`, aktuell 0.2.5 vom 2026-05-16); C-Variante nur mit Feature `bzip2-sys`. Lizenz `bzip2-1.0.6`: Quelltext-Weitergabe muss den Hinweis behalten; Nennung in Produktdokumentation „would be appreciated but is not required“.

**Belege.** https://api.github.com/repos/cisco/openh264/releases (Abruf 2026-10-17); https://www.openh264.org/BINARY_LICENSE.txt; Hash-Berechnung mit `sha256sum`; `openh264-0.9.9/Cargo.toml` (`[features]`), `openh264-sys2-0.9.9/src/lib.rs:143-185`, `openh264-sys2-0.9.9/src/blobs/hashes.txt`; `openh264-sys2-0.9.9/src/generated/types.rs:263` (`ENCODER_OPTION_BITRATE = 5`); https://crates.io/api/v1/crates/bzip2/0.6.1, https://raw.githubusercontent.com/trifectatechfoundation/bzip2-rs/master/Cargo.toml, https://raw.githubusercontent.com/trifectatechfoundation/libbzip2-rs/main/COPYING.

**Folgerung für den Plan:** R16 entfällt (Crate lädt die Cisco-Datei zur Laufzeit; `openh264_abi.rs` ist nicht nötig, außer der Plan will `wide` vermeiden). R4: HTTPS ist verfügbar; Integrität nur über eigene Hash-Pins (SHA-256 oben bzw. Crate-Liste), denn Cisco veröffentlicht nur MD5. Die Server-Zeitstempel (2026-09-01) zeigen, dass Cisco Dateien neu hochlädt; die Inhalte stimmen aber mit der Crate-Liste überein. Lizenzbedingung (4) verlangt den vollständigen Lizenztext an der Lizenzstelle der App (siehe N25).

### N9 — `ashpd` 0.13.13

**Antwort.**
- `zbus = { version = "5.13", default-features = false }` → mit dem vorhandenen `zbus` 5.16.0 verträglich, keine dritte zbus-Version. Pflichtabhängigkeiten außerdem `enumflags2` 0.7 (im Lock), `futures-util`, `getrandom` 0.4 (Lock hat 0.4.2), `serde`; MSRV `rust-version = "1.87"`.
- Feature-Namen ohne Standardfeatures (`default = ["tokio"]`): `screencast`, `remote_desktop` (beide ziehen nur `serde_repr`), `tokio` (→ `zbus/tokio`) oder `async-io`; `pipewire` ist ein eigenes optionales Feature (zieht das Crate `pipewire`) und muss aus bleiben.
- API: `Screencast`/`RemoteDesktop` mit `set_persist_mode`, `set_restore_token`, `restore_token()`; `Screencast::open_pipe_wire_remote` (`src/desktop/screencast.rs:499`); `RemoteDesktop::select_devices` (`src/desktop/remote_desktop.rs:365`), `notify_keyboard_keycode` (`:423`), `notify_pointer_motion_absolute` (`:573`), `connect_to_eis` (`:729`).
- Host-Registrierung: `ashpd::register_host_app(app_id)` ruft `org.freedesktop.host.portal.Registry.Register` (nur ohne Sandbox). xdg-desktop-portal-Doku: Registrierung vor jedem Portalaufruf, nur einmal, App-ID muss dem Basisnamen einer `.desktop`-Datei entsprechen; Interface Version 1. Eingeführt in xdg-desktop-portal 1.19.4 (2025-02-15). Eine Pflicht zur Registrierung für `persist_mode`/`restore_token` nennt keine Primärquelle. xdg-desktop-portal ermittelt seit 1.14.0 die App-ID nicht gesandboxter Programme über libsystemd, seit 1.21.0 mit verbesserter Heuristik.

**Belege.** `ashpd-0.13.13/Cargo.toml` (`[features]`, `[dependencies.zbus]` Zeile 282-284, `rust-version` Zeile 14); `ashpd-0.13.13/src/registry.rs:38-67`; https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.host.portal.Registry.html; https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/main/NEWS.md (letzte Version 1.22.0 vom 2026-06-08).

**Folgerung für den Plan:** Bedingung in C1 („wenn N9 die Verträglichkeit bestätigt“) ist erfüllt. Registrierung ist für Persistenz nicht belegt nötig; wird sie genutzt, ist die App-ID `smart-explorer` (Basisname der installierten Datei `smart-explorer.desktop`, `install-linux.sh:392`) und muss vor dem ersten Portalaufruf derselben D-Bus-Verbindung erfolgen; auf Portalen vor 1.19.4 schlägt der Aufruf fehl und ist dann zu ignorieren.

**Lücke:** Ob xdg-desktop-portal eine App-ID ohne Punkt (`smart-explorer`) bei `Register` annimmt, ist nicht geprüft.

### N10 — PipeWire-C-ABI-Teilmenge, SPA-POD, Bibliotheksname

**Antwort.**
- Bibliotheksname: `libpipewire-0.3.so.0` (`apiversion = '0.3'`, `soversion = 0`, auch im Zweig `master` mit Version 1.7.0). Aktuelle stabile Version 1.6.9 (2026-09-17).
- `struct pw_stream_events` hat `#define PW_VERSION_STREAM_EVENTS 2` und die Felder in dieser Reihenfolge: `version`, `destroy`, `state_changed`, `control_info`, `io_changed`, `param_changed`, `add_buffer`, `remove_buffer`, `process`, `drained`, `command`, `trigger_done`. Das Versionsfeld erlaubt älteren Bibliotheken, nur die ihnen bekannten Felder zu lesen.
- Dokumentierte Aufrufe: `pw_stream_new`, `pw_stream_connect`, `pw_stream_dequeue_buffer`, `pw_stream_queue_buffer` (Header `src/pipewire/stream.h`).
- SPA-POD: Der POD-Builder (`spa/include/spa/pod/builder.h`) ist als `static inline` (`SPA_API_POD_BUILDER`) im Header implementiert und wird **nicht** aus `libpipewire-0.3.so.0` exportiert.

**Belege.** https://gitlab.freedesktop.org/pipewire/pipewire/-/raw/master/meson.build (Zeilen 2, 24-27); `…/src/pipewire/stream.h` (Struktur und Zeile 418); `…/spa/include/spa/pod/builder.h:27-31`; https://gitlab.freedesktop.org/api/v4/projects/pipewire%2Fpipewire/releases.

**Folgerung für den Plan:** Die „eigene schmale Lade-ABI“ (C1 L1, C8) muss den SPA-POD-Aufbau für `EnumFormat` (Video roh, Shared Memory) selbst in Rust kodieren; dafür gibt es kein ladbares Symbol. R3 betrifft damit auch das POD-Format.

**Lücke:** Layout von `struct pw_buffer` und `struct spa_buffer` je Version 0.3.x bis 1.6 nicht einzeln verglichen; Liste der Exporte `pw_init`, `pw_thread_loop_*`, `pw_context_connect_fd` nicht gegen die Symboltabelle geprüft.

### N11 — Portal-Backends: Persistenz, RemoteDesktop, Cursor; nicht interaktive CI

**Antwort (Quelltext der Backends, Zweig `main`/`master` am 2026-10-17):**

| Backend | ScreenCast | RemoteDesktop | `persist_mode`/Wiederherstellung | `AvailableCursorModes` | Fenster als Quelle |
|---|---|---|---|---|---|
| xdg-desktop-portal-gnome | ja | ja | ScreenCast ab 42.rc, RemoteDesktop ab 45.beta („Implement restoration of remote desktop sessions“) | Hidden, Embedded, Metadata | ja (Fenster-Wiederherstellung in NEWS) |
| xdg-desktop-portal-kde | ja | ja | ScreenCast und RemoteDesktop (`persist_mode`, `restore_data` mit `session = "KDE"`); Einführungsversion: Lücke | Hidden, Embedded, Metadata (`Hidden | Embedded | Metadata`) | ja |
| xdg-desktop-portal-wlr (v0.8.4 vom 2026-07-28; Quelltext `master`) | ja | **nein** (`wlr.portal`: nur `Screenshot` und `ScreenCast`) | nur wenn die Umgebungsvariable `XDPW_PERSIST_MODE` (`transient`/`permanent`) gesetzt ist; Standard `PERSIST_NONE` | Hidden, Embedded; Metadata führt zum Abbruch | Monitor und Fenster |

Nicht interaktive CI:
- ScreenCast: `xdg-desktop-portal-wlr` mit `[screencast] chooser_type = none` und `output_name = <Ausgang>` wählt ohne Dialog („xdpw will allow screencast either on the output given by output_name“). Folgerung: kopfloses Sway (wlroots) + xdpw erlaubt ScreenCast ohne Interaktion.
- RemoteDesktop ohne Interaktion: KDE prüft im Permission-Store Tabelle `kde-authorized`, Eintrag `remote-desktop`; steht dort die App-ID (bei Host-Programmen die leere App-ID) mit `yes`, wird ohne Rückfrage freigegeben („mega-authorized … without any further user interaction“). GNOME und wlr: kein solcher Weg gefunden (wlr hat kein RemoteDesktop).

**Belege.** https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/raw/main/NEWS, `…/src/screencast.c:744-754`, `…/src/remotedesktop.c:75-92`; https://invent.kde.org/plasma/xdg-desktop-portal-kde/-/raw/master/data/kde.portal, `…/src/screencast.h:26-64`, `…/src/remotedesktop.cpp:36-73`, `…/src/screencast.cpp:206-289`; https://raw.githubusercontent.com/emersion/xdg-desktop-portal-wlr/master/wlr.portal, `…/src/screencast/screencast.c:161-181`, `:368-399`, `:472`, `:739-742`, `…/xdg-desktop-portal-wlr.5.scd`.

**Folgerung für den Plan:** C5 „Portal-Backend ohne RemoteDesktop → `control_unavailable`“ trifft auf wlroots-Desktops (Sway, Hyprland, river, Wayfire, phosh) immer zu. Persistenz auf wlroots nur mit gesetzter Variable `XDPW_PERSIST_MODE` beim Nutzer; se kann sie nicht setzen. CI: Wayland-Aufnahme mit Sway+xdpw prüfbar; Wayland-Steuern nur mit KDE (KWin kopflos) und Permission-Store-Eintrag prüfbar.

**Lücke:** Plasma-Version der KDE-Wiederherstellung; ob KWin kopflos (`kwin_wayland --virtual`) samt xdg-desktop-portal-kde auf `ubuntu-24.04` startbar ist; GNOME-Version der Fensterquelle.

### N12 — systemd-App-Einheiten, Wayland-Fensterlisten, KWin

**Antwort.**
- Konvention systemd: Dienste `app[-<launcher>]-<ApplicationID>[@<RANDOM>].service`, Scopes `app[-<launcher>]-<ApplicationID>-<RANDOM>.scope`; App-ID durch Abschneiden von Präfix und Suffix, „should map to the corresponding `.desktop` file when available“; Beispiele `app-gnome-org.gnome.Evince@12345.service`, `app-KDE-org.kde.okular@12345.service`. Die Seite sagt nicht ausdrücklich, dass GNOME und KDE die Konvention einhalten.
- `ext-foreign-toplevel-list-v1` (wayland.app, Abruf 2026-10-17): unterstützt von Sway 1.11, Hyprland 0.52.1, niri 26.04, COSMIC 1.0.0~beta.8, Labwc 0.20.2, Jay 1.14.0, Louvre 2.14.1, Mir 2.26, phoc 0.55, river 0.4.8, Treeland 0.8.0; nicht von KWin 6.7, Mutter 51, Wayfire 0.9.0, Weston 15.0.1, Cage 0.3.0, GameScope 3.15.14, Muffin 6.7.4.
- KWin für Drittprogramme: KWin gibt eingeschränkte Wayland-Schnittstellen (darunter `org_kde_plasma_window_management`) einem Prozess frei, wenn dessen `.desktop`-Datei sie unter `X-KDE-Wayland-Interfaces` anfordert; Exec-Zeile mit absolutem Pfad, ohne `%u` (KDE-Mailingliste kwin, Januar 2024; Merge-Request plasma-workspace 1240). Primärquelle KWin-Quelltext für Plasma 6.x nicht gelesen.

**Belege.** https://systemd.io/DESKTOP_ENVIRONMENTS/; https://wayland.app/protocols/ext-foreign-toplevel-list-v1; https://mail.kde.org/pipermail/kwin/2024-January/005305.html und `…/005307.html`; https://invent.kde.org/plasma/plasma-workspace/-/merge_requests/1240.

**Folgerung für den Plan:** Die Aussage „KWin bietet keine Fensterliste für Drittprogramme“ (C5) ist so nicht haltbar: Mit `X-KDE-Wayland-Interfaces=org_kde_plasma_window_management` in `smart-explorer.desktop` (Exec mit absolutem Pfad) ist eine Fensterliste mit Titel, App-ID und PID möglich. Das betrifft die Installationsdatei aus `install-linux.sh:392`. Die Scope-Auswertung muss das optionale `<launcher>`-Segment beachten.

**Lücke:** Aktuelles KWin-Verhalten (6.x) aus dem Quelltext; ob die Freigabe auch für einen Daemon-Prozess gilt, dessen Programmdatei `se` statt `smart_explorer` ist (KWin ordnet über den Exec-Pfad zu — Annahme).

### N13 — X11: Cursor, EWMH, XTest-Text, MIT-SHM mit FD

**Antwort.**
- `x11rb` 0.13.2 hat die Features `shm`, `damage`, `xfixes`, `randr`, `composite`, `xtest`. `xfixes::get_cursor_image` (`src/protocol/xfixes.rs:100`) und `get_cursor_image_and_name` (`:407`) vorhanden. `shm::attach_fd(conn, shmseg, shm_fd, read_only)` (`src/protocol/shm.rs:242`) und `shm::create_segment` (`:269`, Antwort mit FD) vorhanden; `RustConnection` überträgt FDs über das Unix-Socket (`src/rust_connection/stream.rs:1-5`, `:74`).
- XTest-Text über freien Keycode, Verfahren nach xdotool (`xdo.c:1047-1113`): Tastaturbelegung mit `XGetKeyboardMapping` von `min_key_code` bis `max_key_code` lesen, einen Keycode ohne Keysyms als Arbeits-Keycode wählen, mit `XChangeKeyboardMapping` das gewünschte Keysym darauf legen, Taste drücken/loslassen, danach die Belegung wieder zurücksetzen. xdotool steht unter **BSD-3-Clause** (nicht GPL).
- EWMH: nicht neu abgerufen — Lücke (Spezifikation `_NET_CLIENT_LIST`, `_NET_WM_NAME`, `_NET_WM_PID`, `_NET_ACTIVE_WINDOW` nicht zitiert).

**Belege.** `x11rb-0.13.2/Cargo.toml` (`[features]`); Dateien oben; https://raw.githubusercontent.com/jordansissel/xdotool/master/xdo.c; https://api.github.com/repos/jordansissel/xdotool/license (`BSD-3-Clause`).

**Folgerung für den Plan:** Der Quellenhinweis in G N13 („GPL-Code nicht übernehmen“) beruht auf falscher Lizenzannahme; das Verfahren darf mit BSD-3-Hinweis übernommen werden.

### N14 — VA-API

**Antwort (unvollständig).** RPM Fusion liefert für Fedora das Paket `mesa-va-drivers-freeworld` „for decoding/encoding H.264 and H.265“ (Rawhide: 26.1.2-1.fc45, 2026-06-03), d. h. die Fedora-eigenen Mesa-VA-Treiber kodieren H.264 nicht. `libva` ist MIT-lizenziert (Ref B1).

**Belege.** https://download1.rpmfusion.org/free/fedora/development/rawhide/Everything/x86_64/os/repoview/mesa-va-drivers-freeworld.html.

**Lücke:** Ladbare libva-Teilmenge (`vaGetDisplayDRM`, `vaInitialize`, `vaQueryConfigEntrypoints`, `vaCreateConfig`, `vaCreateContext`, `vaCreateBuffer` für gepackte Köpfe) nicht gegen den Header geprüft; Intel-iHD-Entrypoint `VAEntrypointEncSliceLP` je Generation nicht belegt; DMA-BUF-Import (`vaCreateSurfaces` mit `VASurfaceAttribExternalBuffers`/`DRM_PRIME_2`) nicht belegt.

**Folgerung für den Plan:** SCR-M10 bleibt bedingt; Go/No-Go ist mit dieser Recherche nicht entscheidbar.

### N15 — Android: API-Level, Hintergrundstart, `appops`, FGS, Emulator-Kodierer

**Antwort.**
- API-Level (developer.android.com, Abruf 2026-10-17): `KEY_LOW_LATENCY` 30 („applies only to decoders“), `KEY_MAX_B_FRAMES` 29 (Standard 0), `KEY_LATENCY` 26 („applies only to video encoders … ignored if the video encoder does not support“), `KEY_PRIORITY` 23, `KEY_PREPEND_HEADER_TO_SYNC_FRAMES` 29, `KEY_INTRA_REFRESH_PERIOD` 24, `KEY_REPEAT_PREVIOUS_FRAME_AFTER` 19, `KEY_BITRATE_MODE` 21, `MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME` 19, `PARAMETER_KEY_VIDEO_BITRATE` 19, `CodecProfileLevel.AVCProfileConstrainedBaseline` 27, `MediaCodecInfo.isHardwareAccelerated` 29, `ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION` 29. Alle ≤ `minSdk = 30`.
- Zustimmungsaktivität aus Benachrichtigung: Ausnahme der Hintergrundstartsperre: „The activity is started from a `PendingIntent` that was sent by the system (for example, from a notification tap).“ Für Apps mit Ziel API 34+ gewährt der **Sender** eines `PendingIntent` seine Rechte nicht mehr automatisch; für Ziel 35+ der **Ersteller** ebenfalls nicht (`setPendingIntentCreatorBackgroundActivityStartMode`). Die Seite nennt Benachrichtigungs-**Aktionen** nicht ausdrücklich.
- `appops`: SystemUI `MediaProjectionPermissionActivity` überspringt den Dialog, wenn `hasProjectionPermission(uid, package)` wahr ist; der Dienst prüft dafür `noteOpNoThrow(OP_PROJECT_MEDIA, …) == MODE_ALLOWED`. Folgerung: `adb shell appops set <Paket> PROJECT_MEDIA allow` überspringt den Dialog im Emulator.
- Vordergrunddienst: Typ `mediaProjection`, Berechtigung `FOREGROUND_SERVICE_MEDIA_PROJECTION`, Konstante `FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION`; „Call the `createScreenCaptureIntent()` method before starting the foreground service … the user must grant the permission before you can create the service.“ Ab Android 14 Typ für jeden FGS Pflicht; Ziel Android 15: kein mediaProjection-FGS aus `BOOT_COMPLETED`.
- Latenz von `c2.android.avc.encoder` im Emulator: **Lücke** (nur messbar).

**Belege.** https://developer.android.com/reference/android/media/MediaFormat, `…/media/MediaCodec`, `…/media/MediaCodecInfo`, `…/media/MediaCodecInfo.CodecProfileLevel`, `…/content/pm/ServiceInfo`; https://developer.android.com/guide/components/activities/background-starts; https://developer.android.com/develop/background-work/services/fg-service-types; AOSP-Spiegel `aosp-mirror/platform_frameworks_base` Zweig `main` (letzter Commit 1cdfff55, 2025-03-27): `packages/SystemUI/src/com/android/systemui/mediaprojection/permission/MediaProjectionPermissionActivity.java:155-181`, `services/core/java/com/android/server/media/projection/MediaProjectionManagerService.java:714-722`.

**Folgerung für den Plan:** Die „Freigeben“-Schaltfläche muss ein `PendingIntent.getActivity` auf die Zustimmungsaktivität sein (kein Dienst/Broadcast als Zwischenschritt); bei Ziel 36 ist `setPendingIntentCreatorBackgroundActivityStartMode(MODE_BACKGROUND_ACTIVITY_START_ALLOWED)` beim Erstellen zu setzen (Annahme: für Benachrichtigungsaktionen nötig, Primärquelle schweigt). Die Plan-Schlüssel sind ab `minSdk` 30 verfügbar.

**Lücke:** Der AOSP-Spiegel endet 2025-03-27; Android-16-Stand der SystemUI-Stelle nicht geprüft.

### N16 — Android-Bedienungshilfe

**Antwort.**
- `takeScreenshot` (API 30): Mindestabstand `ACCESSIBILITY_TAKE_SCREENSHOT_REQUEST_INTERVAL_TIMES_MS = 333` (AOSP, verborgene Konstante); zu früh → `ERROR_TAKE_SCREENSHOT_INTERVAL_TIME_SHORT` (3). Fehlercodes: `INTERNAL_ERROR`, `NO_ACCESSIBILITY_ACCESS`, `INTERVAL_TIME_SHORT`, `INVALID_DISPLAY` (alle API 30), `INVALID_WINDOW` und `SECURE_WINDOW` (beide API 34). Pflicht-Metadatum `canTakeScreenshot`.
- `ERROR_TAKE_SCREENSHOT_SECURE_WINDOW` gehört zu `takeScreenshotOfWindow` (API 34); `takeScreenshot` für die ganze Anzeige kennt diesen Fehler nicht (AOSP `AbstractAccessibilityServiceConnection.takeScreenshot` sendet nur `INTERVAL_TIME_SHORT`, `INTERNAL_ERROR`, `NO_ACCESSIBILITY_ACCESS`, `INVALID_DISPLAY`). Annahme: `FLAG_SECURE`-Inhalte erscheinen in der Gesamtaufnahme schwarz.
- `getWindows()` API 21, `AccessibilityWindowInfo.getTitle()` API 24; `dispatchGesture` API 24; `StrokeDescription.continueStroke` API 26; `GLOBAL_ACTION_RECENTS`/`NOTIFICATIONS` 16, `QUICK_SETTINGS` 17, `POWER_DIALOG` 21, `LOCK_SCREEN` 28; `AccessibilityAction.ACTION_IME_ENTER` 30.
- „Eingeschränkte Einstellungen“: Google-Hilfe nennt Bedienungshilfe als Beispiel, Freigabe über App-Info → Menü → „Allow restricted settings“, „Some of these steps work only on Android 13 and up“; für 14/15/16 keine Angaben. Sekundärquellen: Android 13 entscheidet nach Installationsweg (nicht sitzungsbasierte Installer gelten als Sideload); Android 15 „Enhanced Confirmation Mode“ verschärft das (Android Authority, Analyse der Beta).
- Aktivierung im Emulator per `settings put secure enabled_accessibility_services`: **Lücke** (keine Primärquelle abgerufen; `adb shell` hat `WRITE_SECURE_SETTINGS` — Annahme).

**Belege.** https://developer.android.com/reference/android/accessibilityservice/AccessibilityService, `…/GestureDescription.StrokeDescription`, `…/view/accessibility/AccessibilityWindowInfo`, `…/view/accessibility/AccessibilityNodeInfo.AccessibilityAction`; AOSP `core/java/android/accessibilityservice/AccessibilityService.java:788`, `services/accessibility/java/com/android/server/accessibility/AbstractAccessibilityServiceConnection.java:1467-1530`; https://support.google.com/android/answer/12623953; https://www.androidauthority.com/android-15-enhanced-confirmation-mode-3436697/.

**Folgerung für den Plan:** C6-Vorschau mit 1 s Abstand liegt über der Systemgrenze 333 ms (≤ 3 Bilder/s). Die Begründung in C1 L2 „`FLAG_SECURE`-Fenster liefern Fehler“ ist für die Gesamtaufnahme falsch (siehe Abschnitt 2).

### N17 — Android unbeaufsichtigt (Shizuku, scrcpy, `screenrecord`)

**Antwort.**
- Shizuku-API: `UserService` führt eigenen Java- oder JNI-Code „in a different process and as the identity (Linux UID) of root (UID 0) or shell (UID 2000 …)“ aus, angebunden über Binder (`bindUserService`); `newProcess` ist zur Entfernung vorgesehen („`UserService` can replace `newProcess` in all cases“). Folgerung: Die Frage nach der Erreichbarkeit eines Abstract-Sockets des Shell-Prozesses aus der App (SELinux `untrusted_app` → `shell`) entfällt, weil der Datenweg Binder bzw. ein per Binder übergebener Dateideskriptor ist.
- `screenrecord` (AOSP `frameworks/av` Zweig `main`): `--output-format` kennt `mp4`, `h264`, `webm`, `3gpp`, `frames`, `raw-frames`; `kMaxTimeLimitSec = 180`; `--time-limit 0` hebt die Grenze auf. B-Bilder Standard 0 (`gBframes = 0`).
- scrcpy-Muster: Ref C4/E1 (Apache-2.0).

**Belege.** https://raw.githubusercontent.com/RikkaApps/Shizuku-API/master/README.md (Abschnitt „UserService“, Änderungsnotiz zu `newProcess`); https://android.googlesource.com/platform/frameworks/av/+/refs/heads/main/cmds/screenrecord/screenrecord.cpp (Zeilen 102, 114, 124-125, 1232-1234, 1322-1336).

**Lücke:** Lizenz der Shizuku-API-Bibliothek nicht abgerufen; ab welcher Android-Version `screenrecord --time-limit 0` gilt; Latenz von `screenrecord`.

### N18 — Windows: Integritätsstufe, `SendSAS`, SYSTEM-Dienst

**Antwort.**
- `SendSAS(AsUser)`: Aufrufer muss „either be running as a service or have the uiAccess attribute … set to "true"“; ohne Dienst nur als aktueller Nutzer oder LocalSystem und nur mit aktivem UAC; UIAccess-Programme Authenticode-signiert in `\Program Files\` oder `\windows\system32\`; Richtlinie „Computer Configuration | Administrative Templates | Windows Components | Windows Logon Options | Disable or enable software Secure Attention Sequence“; Mindestclient Windows 7, `Sas.dll`. Ein Dienst kann das Token eines aufrufenden Prozesses imitieren, dann gilt SAS für dessen Sitzung.
- Integritätsstufe des Vordergrundfensters (`GetForegroundWindow` → `GetWindowThreadProcessId` → `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `OpenProcessToken(TOKEN_QUERY)` → `GetTokenInformation(TokenIntegrityLevel)`): **Lücke** — ob `OpenProcessToken` aus Medium IL auf einen erhöhten Prozess desselben Nutzers gelingt, ist nicht mit Primärquelle belegt. Gegenmaßnahme (Folgerung): scheitert die Abfrage, gilt das Fenster als „höher“ (`blocked: "elevated_window"`).
- Muster SYSTEM-Dienst + Helfer auf dem Winlogon-Desktop: keine Microsoft-Quelle gefunden (Lücke bleibt).

**Belege.** https://learn.microsoft.com/en-us/windows/win32/api/sas/nf-sas-sendsas (updated_at 2025-08-27); `windows-0.58.0/src/Windows/Win32/Security/Authentication/Identity/mod.rs:1202`.

**Folgerung für den Plan:** Strg+Alt+Entf ist auch über einen SYSTEM-Dienst nur mit gesetzter Richtlinie „software SAS“ möglich; der TODO-Eintrag muss diese Richtlinie nennen.

### N19 — Iroh-Relay mit Video

**Antwort.**
- Der Relay-Server **verwirft** Pakete, wenn die Ziel-Warteschlange voll ist: `try_send_packet` reserviert Bytes aus dem Warteschlangenbudget und meldet `TrySendPacketError::Full`, wenn Budget (1 MiB je Ziel) oder Kanal (`PER_CLIENT_SEND_QUEUE_DEPTH = 512`) erschöpft ist; der Aufrufer protokolliert „client too busy to receive packet, dropping packet“. Gleiches im unveränderten iroh-relay 1.3.0.
- Der Relay-**Client** in iroh 1.3.0 verwirft beim Neuaufbau der Relay-Verbindung regelmäßig wartende Datagramme („Those datagrams are dropped and the QUIC congestion controller will have to handle this“).
- Pro Relay-Verbindung Empfangsrate 64 MiB/s (`share-server/src/relay.rs:23`).
- Pfadwechsel: iroh 1.3.0 nutzt QUIC-Multipath mit `MAX_MULTIPATH_PATHS = 8`, `HEARTBEAT_INTERVAL = 5 s`, `PATH_MAX_IDLE_TIMEOUT = 15 s`. Latenzspitzen beim Wechsel: **Lücke** (nur messbar).

**Belege.** `V/iroh-relay-1.0.0/src/server/client.rs:242-258`, `V/iroh-relay-1.0.0/src/server/clients.rs:263-268`, `V/iroh-relay-1.0.0/src/protos/relay.rs:40`; `iroh-relay-1.3.0/src/protos/relay.rs:40`, `iroh-relay-1.3.0/src/server/clients.rs:212-224`; `iroh-1.3.0/src/socket/transports/relay/actor.rs:451-460`; `iroh-1.3.0/src/socket.rs:109`, `:117`, `:137`; `docs/SHARE_SERVER.md:259-264`; `share-server/Cargo.toml:16-32` (vendored iroh-relay 1.0.0).

**Folgerung für den Plan:** Über das Relay entsteht echter QUIC-Paketverlust, sobald die Ziel-Warteschlange (1 MiB ≈ 1,4 s bei 6 Mbit/s) voll ist. Die verlustbasierte Regelung (C3 (2)) greift damit auch auf dem Relay-Pfad; der Warteschlangenwächter (C3 (3)) muss vor 1 MiB unbestätigter Daten greifen, damit das Relay nicht verwirft.

### N20 — wgpu 22 auf GL: `R8Unorm`/`Rg8Unorm`, `write_texture`

**Antwort.** GL-Abbildung `R8Unorm` → `(GL_R8, GL_RED, GL_UNSIGNED_BYTE)`, `Rg8Unorm` → `(GL_RG8, GL_RG, GL_UNSIGNED_BYTE)`; Fähigkeit beider Formate `filterable_renderable`. `COPY_BYTES_PER_ROW_ALIGNMENT = 256` gilt ausdrücklich nicht für `Queue::write_texture` („This doesn't apply to `Queue::write_texture`“).

**Belege.** `wgpu-hal-22.0.0/src/gles/conv.rs:10`, `:19`; `wgpu-hal-22.0.0/src/gles/adapter.rs:1067`, `:1076`; `wgpu-types-22.0.0/src/lib.rs:77-81`.

**Folgerung für den Plan:** C4 L1 (Y als `R8Unorm`, UV als `Rg8Unorm`, Upload per `write_texture` mit beliebigem Zeilenabstand) gilt auch auf dem GL-Rückfall.

### N21 — GitHub-Linux-Runner

**Antwort.** `ubuntu-24.04` (Image 20261012.334.1): `xvfb` 2:21.1.12-1ubuntu1.8 und `iproute2` 6.1.0-1ubuntu6.4 sind installiert; PipeWire ist in der Paketliste nicht aufgeführt. MIT-SHM und XTEST in Xvfb: nicht ausdrücklich belegt (Annahme: in Xorg-Server 21.1 einkompiliert). `tc netem` auf `lo`: **Lücke** — ob das Modul `sch_netem` im Runner-Kernel ladbar ist, ist nicht belegt; netem wirkt auf der Ausgangsseite einer Schnittstelle, auf `lo` also in beiden Richtungen (Folgerung: eingestellte Verzögerung zählt doppelt in der Umlaufzeit).

**Belege.** https://raw.githubusercontent.com/actions/runner-images/main/images/ubuntu/Ubuntu2404-Readme.md (Zeilen 11, 283, 329); `…/images/ubuntu/toolsets/toolset-2404.json`; https://www.mankier.com/8/tc-netem.

### N22 — Wie findet `se` das GUI-Binary?

**Antwort.**
- Namen: Windows `installed_app_names() = ["Smart Explorer.exe", "smart_explorer.exe"]` (`N/updater/os/windows.rs:109-111`), Linux `["smart_explorer"]` (`N/updater/os/linux_os.rs:8`, `:102-104`). Das Binary heißt **`smart_explorer`** (Paketname `native/Cargo.toml:2`), installiert unter Windows als `Smart Explorer.exe` in `%LOCALAPPDATA%\Programs\Smart Explorer` (`native/installer.nsi:47`, `:60`), unter Linux als `$HOME/.local/opt/smart-explorer/smart_explorer` mit Verweis `~/.local/bin/smart_explorer` (`install-linux.sh:8-15`, `:369`).
- Verfahren: `updater::…::detect_installation()` nimmt den kanonisierten Pfad des laufenden `se` (`running_cli_path`, Linux `fs::canonicalize`, Windows `current_exe`) und sucht im selben Ordner nach den App-Namen (`N/updater/os/shared/terminal.rs:63-80`, `N/updater/os/linux_os.rs:111-115`, `N/updater/os/windows.rs:118-120`). Entwicklungsstände: `target/<profil>/se` und `target/<profil>/smart_explorer` liegen nebeneinander → gefunden.
- Der Daemon: Windows-Autostart startet immer das GUI-Binary neben `se.exe` (`logon_exe_for`, `N/autostart/os/windows.rs:12-22`, `:30-37`); Linux-Autostart startet das aufrufende Programm selbst (`daemon_exe_for(exe) = exe`, `N/autostart/os/linux_os.rs:56-61`), also `se` oder `smart_explorer`.

**Folgerung für den Plan:** Die Plan-Schreibweise `smart-explorer --screen-view|--screen-consent|--screen-indicator` muss `smart_explorer` bzw. `Smart Explorer.exe` heißen. Ein Daemon, der als `se` läuft (Linux), findet das GUI-Binary nur über den Geschwister-Ordner; fehlt es (`Installation::TerminalOnly`), gibt es kein Zustimmungs-/Anzeigefenster → der Plan braucht dafür einen Zustand (Annahme: Ablehnung von `ask`-Anfragen mit Grund).

### N23 — Windows-Mindestversion und eingebaute RDP-Hosts

**Antwort.**
- Smart Explorer nennt keine allgemeine Windows-Mindestversion; dokumentiert ist nur Exec-Host über ConPTY „ab Windows 10 1809 bzw. Server 2019“ (`README.md:826`, `:855`). Kein Manifest-`supportedOS`, keine Build-Prüfung im Code (Suche `supportedOS|MinVersion|18362|17763` ohne Treffer). Rust ab 1.78: „Windows 10 will now be the minimum supported version for the `*-pc-windows-*` targets“; CI nutzt `dtolnay/rust-toolchain@stable` (`.github/workflows/build.yml:129`).
- Windows-RDP-Host: nur Pro und höher (Ref E2). Dass eine eingehende RDP-Sitzung die Konsolensitzung trennt/sperrt und nur eine Sitzung erlaubt ist, belegen nur Sekundärquellen (Windows OS Hub); keine Microsoft-Primärquelle gefunden.

**Belege.** Dateien oben; https://blog.rust-lang.org/2024/02/26/Windows-7/; https://woshub.com/how-to-allow-multiple-rdp-sessions-in-windows-10/.

**Folgerung für den Plan:** Unterstützt sind faktisch Windows 10 ab RTM; WGC-Interop braucht 1903 → DXGI-Rückfall für Windows 10 1507–1809 (darunter LTSC 2015/2016/2019) ist nötig wie geplant.

### N24 — Windows 11: programmatische WGC-Aufnahme aus unverpacktem Prozess

**Antwort.**
- `IGraphicsCaptureItemInterop::CreateForWindow` und `CreateForMonitor`: Mindestclient „Windows 10 Version 1903 (Build 18362)“; die Referenzseiten nennen **keine** Zustimmungs- oder Paketpflicht.
- `GraphicsCaptureItem.TryCreateFromDisplayId` (ab 10.0.20348.0): „Before attempting to create a GraphicsCaptureItem with this method, you must call `GraphicsCaptureAccess.RequestAccessAsync`, passing in … `Programmatic`. To call **RequestAccessAsync** with … Programmatic, you must declare the **graphicsCaptureProgrammatic** capability in your app's package manifest.“ Die Pflicht ist also an die WinRT-Fabriken `TryCreateFromDisplayId`/`TryCreateFromWindowId` gebunden.
- Gelber Rahmen: Abschalten nur nach `RequestAccessAsync(Borderless)` mit Paket-Capability `graphicsCaptureWithoutBorder`; „If the user denies access, setting this property to false will succeed, but the value will be ignored“.

**Belege.** https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createformonitor und `…-createforwindow`; https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureitem.trycreatefromdisplayid (updated_at 2025-11-21); https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired (updated_at 2026-07-23).

**Folgerung für den Plan:** C1 L1 muss die Interop-Fabriken (`CreateForMonitor`/`CreateForWindow`) verwenden, nicht `TryCreateFromDisplayId`/`TryCreateFromWindowId`; dann ist keine Paketidentität nötig (Annahme: Verhalten auf 24H2/25H2 entspricht der Doku; nicht gemessen). Der Rahmen bleibt, weil se keine Paket-Capability hat.

**Lücke:** Microsoft-Aussage zum Verhalten unverpackter Prozesse bei `RequestAccessAsync`.

### N25 — Lizenztexte im Release

**Antwort.**
- Windows-Installer: legt `LICENSE` (eigene MIT-Lizenz) und unter `licenses\Dokany\` `NOTICE.txt`, `LICENSE-GPL-3.0.txt`, `LICENSE-LGPL-3.0.txt`, `LICENSE-MIT.txt` ab (`native/installer.nsi:134-139`); Installer-Seite zeigt `DISCLAIMER.txt` (`:69-73`).
- `native/publish-release-local.ps1` enthält kein Vorkommen von „licen“ (Zählung 0); `install-linux.sh` enthält kein Vorkommen; das Update-Feed (`release-native/update-feed/`) enthält keine Lizenzdateien; `third-party/` enthält nur `dokany`. `docs/RELEASING.md:100-113` regelt nur Dokany-Texte.
- Es gibt keinen Mechanismus (zum Beispiel `cargo-about`), der Lizenztexte der Rust-Crates verteilt (Suche nach `about.toml|cargo-about|deny.toml|THIRD_PARTY|third_party_licenses` ohne Treffer).
- Pflichten der neuen Bausteine: `ashpd` (MIT) verlangt den Copyright- und Lizenzhinweis „in all copies or substantial portions“ (MIT-Wortlaut, Annahme: Binärverteilung eingeschlossen); `libbz2-rs-sys` (`bzip2-1.0.6`) verlangt bei Binärverteilung keinen Hinweis; `openh264` (BSD-2-Clause) verlangt bei Binärverteilung die Wiedergabe des Hinweises in der Dokumentation; die Cisco-Binärlizenz verlangt (Bedingung 4) den vollständigen Text an der Lizenzstelle der App.

**Belege.** Dateien oben; Lizenztexte siehe N8.

**Folgerung für den Plan:** Die Lücke ist bestehend und betrifft alle MIT/Apache/BSD-Crates, nicht nur `ashpd`. Für S4 mindestens nötig: Cisco-Binärlizenztext und `openh264`-BSD-Hinweis in der App (Lizenzstelle) und in der Release-Dokumentation; Eintrag für `ashpd`. Ob der Plan die allgemeine Crate-Lizenzlücke im Stapel schließt, ist Planentscheidung.

---

## 2. Revalidierung Abschnitt C

Stelle = Abschnitt und Lösung in `strang-bildschirm.md`. Ergebnis: **bestätigt**, **widerlegt** (mit richtiger Angabe) oder **nicht belegbar** (mit genauer Lücke). „Ref“ = `docs/refs/bildschirm-streaming-2026-10-17.md`; Ref-Belege wurden übernommen, wenn ihre Primärquelle dort mit Abrufdatum 2026-10-17 zitiert ist.

| Nr. | Aussage im Plan (Stelle) | Ergebnis | Beleg | Richtige Angabe / genaue Lücke |
|---|---|---|---|---|
| 1 | WGC über `windows` 0.58 mit `CreateForMonitor`, `CreateForWindow`, freilaufendem Rahmenpool (C1 Windows L1) | bestätigt | `windows-0.58.0/Cargo.toml` Features `Graphics_Capture`, `Win32_System_WinRT_Graphics_Capture`, `Win32_System_WinRT_Direct3D11`; `…/Graphics/Capture/mod.rs:326` | Zusätzlich nötig: `Graphics_DirectX_Direct3D11`, `Win32_Graphics_Direct3D11`, `Win32_Graphics_Dxgi` (N5) |
| 2 | WGC ab Windows 10 1903; Cursor-Schalter ab 2004 (C1 L1) | bestätigt | Interop-Referenz „Windows 10 Version 1903 (Build 18362)“ (N24); Ref A1 `IsCursorCaptureEnabled` 10.0.19041.0 | — |
| 3 | Programmatische WGC-Aufnahme ohne Zustimmung/Paket (implizit C1 L1, N24) | bestätigt mit Einschränkung | N24 | Gilt für `IGraphicsCaptureItemInterop::CreateForMonitor/CreateForWindow`; `TryCreateFromDisplayId/WindowId` verlangen `RequestAccessAsync(Programmatic)` mit Paket-Capability |
| 4 | DXGI Desktop Duplication ab Windows 8, nur ganze Ausgänge (C1 L2) | bestätigt | `DuplicateOutput`-Referenz „Windows 8 and Platform Update for Windows 7“ (Abruf 2026-10-17); Ref A2 | — |
| 5 | `windows-capture` 2.0.1 MIT bringt eigene `windows`-Version (Annahme ≥ 0.6x), „zweite große Bindungsschicht neben 0.58“ (C1 L4) | widerlegt (Kostenbegründung) | `windows-capture-2.0.1/Cargo.toml:81-82` `windows = "0.62.2"`; `native/Cargo.lock`: `windows` 0.62.2 ist bereits im Lock über `netwatch` 0.19.3, `pageant` 0.2.1, `wmi` 0.18.4 | Version 0.62.2 bestätigt; eine zweite Bindungsschicht entsteht nicht neu, sie ist bereits im Lock. Die Verwerfung stützt sich damit nur noch auf „kein Funktionsgewinn“ |
| 6 | Minimierte Fenster liefern keine Bilder (C1 Wahl) | nicht belegbar | Ref A1 (nur WebRTC-Quelltextkommentar) | Lücke: keine Microsoft-Primärquelle |
| 7 | Gelber Rahmen bleibt, se fordert `Borderless` nie an (C1 Wahl, C7) | bestätigt | `IsBorderRequired`-Referenz (updated_at 2026-07-23) | Abschalten nur mit Paket-Capability `graphicsCaptureWithoutBorder`; für se ohne Paket nicht möglich |
| 8 | Sicherer Desktop aus Nutzerprozess nicht lesbar (C1 Wahl) | bestätigt für DXGI, nicht belegbar für WGC | Ref A2 („only an application that runs at LOCAL_SYSTEM can access the secure desktop“) | Lücke: WGC-Verhalten undokumentiert (N5) |
| 9 | Zwei 4K-Monitore (7680 × 2160) überschreiten 36 864 Makroblöcke (C1 Wahl) | bestätigt | 7680/16 × 2160/16 = 480 × 135 = 64 800 > 36 864 | — |
| 10 | Portal ScreenCast: Monitor und Fenster, `persist_mode` 2, einmaliger `restore_token`; GNOME, KDE, wlroots (C1 Linux L1) | bestätigt mit Einschränkung | N11 | wlroots (xdpw): Persistenz nur mit Nutzervariable `XDPW_PERSIST_MODE`; kein RemoteDesktop |
| 11 | `pipewire` 0.10.1 MIT; bindgen/pkg-config beim Build; Programm startet ohne PipeWire nicht (C1 L2) | bestätigt | crates.io-API: `pipewire-sys` 0.10.1 Build-Abhängigkeiten `bindgen ^0.72.0`, `system-deps ^7.0.5`; Lizenz MIT | Startabbruch ohne Bibliothek ist Folgerung aus dynamischem Linken, nicht ausgeführt |
| 12 | Eigene schmale Lade-ABI für PipeWire über `libloading` (C1 L1, C8) | bestätigt mit Mehraufwand | N10 | SPA-POD-Builder ist `static inline` im Header, nicht exportiert → POD-Kodierung in Rust nötig; Bibliothek `libpipewire-0.3.so.0` |
| 13 | X11 über `x11rb` 0.13.2 mit MIT-SHM, DAMAGE, XFixes, RandR, Composite (C1 L3) | bestätigt | `x11rb-0.13.2/Cargo.toml` Features `shm`, `damage`, `xfixes`, `randr`, `composite`, `xtest`; Lizenz MIT OR Apache-2.0 | — |
| 14 | KMS/DRM braucht `CAP_SYS_ADMIN` (C1 L5) | nicht belegbar | — | Lücke: keine Primärquelle abgerufen (betrifft nur eine verworfene Lösung) |
| 15 | `ashpd` 0.13 (MIT, `zbus` 5) nutzbar, wenn N9 passt (C1 Wahl, C8) | bestätigt | N9: `zbus ^5.13`, Lizenz MIT, 0.13.13 ist neueste Version | Feature `pipewire` nicht aktivieren; MSRV 1.87 |
| 16 | Cursor-Modus Embedded, wenn angeboten (C1 Wahl) | bestätigt | N11: GNOME/KDE Hidden+Embedded+Metadata, xdpw Hidden+Embedded | — |
| 17 | Android MediaProjection ab API 21, FGS-Typ ab API 29, Zustimmung je Sitzung Pflicht ab 14, Ende bei Sperre (C1 Android L1) | bestätigt | `ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION` API 29; Ref A5 | Zusätzlich: Zustimmung vor `startForeground` (N15) |
| 18 | Einzelne App ab Android 14 QPR2 (C1 L1) | bestätigt | Ref A5 | — |
| 19 | `takeScreenshot` (API 30) als Standbildquelle; „`FLAG_SECURE`-Fenster liefern Fehler“ (C1 Android L2) | widerlegt (zweiter Teil) | N16: `ERROR_TAKE_SCREENSHOT_SECURE_WINDOW` ist API 34 und gehört zu `takeScreenshotOfWindow`; `takeScreenshot` der Anzeige meldet ihn nicht | Gesamtaufnahme liefert ein Bild; sichere Inhalte erscheinen nach Annahme schwarz. Mindestabstand 333 ms (≤ 3 Bilder/s) |
| 20 | Patentlizenz der OS-Codecs beim OS-Hersteller (C2 L1, Annahme) | nicht belegbar | — | Lücke: keine Primärquelle von Microsoft oder Google gefunden (R12 bleibt offen) |
| 21 | OpenH264 nur als gesondert geladene Cisco-Binärdatei patentgedeckt; Anzeige „OpenH264 Video Codec provided by Cisco Systems, Inc.“; Nutzer schaltet ein/aus (C2 L1, C8) | bestätigt, unvollständig | `BINARY_LICENSE.txt` v1.0 | Zusätzlich Bedingung 4: vollständiger Lizenztext samt dieser Bedingung in EULA bzw. an der Lizenzstelle der App; AVC-Lizenz nur für persönliche, nicht vergütete Nutzung |
| 22 | `lz4_flex` MIT, im Lock (C2 L2, C8) | bestätigt | Lock 0.14.0; crates.io-Lizenz MIT | — |
| 23 | HEVC-Dekodierung auf Windows braucht Store-Erweiterung (C2 L6) | nicht belegbar | Ref „Lücke 5“ | Lücke: keine Microsoft-Primärquelle |
| 24 | Windows-Kodierer: Hardware-MFT per `MFTEnumEx` + `MFT_ENUM_FLAG_HARDWARE`, asynchron → `Mfh264enc.dll` (C2 Kodierer) | bestätigt | `windows-0.58.0/src/Windows/Win32/Media/MediaFoundation/mod.rs:1705`; Ref B1 (Hardware-MFTs asynchron) | — |
| 25 | Windows-N-Editionen ohne Media Foundation (C2 Kodierer) | nicht belegbar | Microsoft Media-Feature-Pack-Seite (N6) | Microsoft nennt „media-related technologies“, Media Foundation nicht namentlich |
| 26 | Android-Kodiererwahl über `MediaCodecInfo.isHardwareAccelerated` (API 29) (C2) | bestätigt | N15 | — |
| 27 | Windows-Decoder MF-H.264 mit DXVA, Ausgabe NV12 (C2 Dekodierer) | bestätigt | MF-H.264-Decoder-Seite (N6) | Profile Baseline/Main/High bis Level 5.1, max. 4096 × 2304; DXVA garantiert bis 1920 × 1088 |
| 28 | „OpenH264 dekodiert nur Constrained Baseline“; Linux-Betrachter erhält `cb` (C2 Dekodierer und Aushandlung) | widerlegt | OpenH264 `RELEASES`: v1.5.0 „Decoder support of 'Constrained High Profile'“ (CABAC, Intra 8×8, 8×8-Transform, Skalierungsmatrizen, Gewichtete Prädiktion); v2.0.0 „B-frame decoding support for Main and High Profile“. Widerspruch: README nennt beim Decoder nur „Constrained Baseline Profile up to Level 5.2“ | Ein progressiver `high`-Strom ohne B-Bilder (wie in C2 verlangt) ist Constrained High und laut Release-Notizen von OpenH264 dekodierbar; Lücke: kein Test, README widerspricht |
| 29 | Niedriglatenz-Schalter MF (`CODECAPI_AVEncMPVDefaultBPictureCount`, `CODECAPI_AVLowLatencyMode`, `CODECAPI_AVEncVideoForceKeyFrame`, `CODECAPI_AVEncCommonMeanBitRate`) (C2) | bestätigt | GUIDs in `windows-0.58.0/…/MediaFoundation/mod.rs:27137`, `:27209`, `:27270`, `:27327`; Ref B1 | — |
| 30 | MediaCodec `KEY_MAX_B_FRAMES = 0`, `KEY_PRIORITY = 0`, `KEY_LATENCY = 1`, Decoder `KEY_LOW_LATENCY = 1`, `PARAMETER_KEY_REQUEST_SYNC_FRAME`, `PARAMETER_KEY_VIDEO_BITRATE`, `KEY_PREPEND_HEADER_TO_SYNC_FRAMES` (C2) | bestätigt | API-Level 29, 23, 26, 30, 19, 19, 29 (N15) | `KEY_LATENCY` und `KEY_LOW_LATENCY` werden ignoriert, wenn der Codec sie nicht kann (R7) |
| 31 | OpenH264 `ENCODER_OPTION_BITRATE` zur Laufzeit (C2) | bestätigt | `openh264-sys2-0.9.9/src/generated/types.rs:263` | — |
| 32 | Größte Ausgabe 4096 × 2304 = 36 864 Makroblöcke, auch OpenH264 (C2 Grenzen) | bestätigt | OpenH264-README (Encoder und Decoder „Level 5.2 (Max frame size is 36864 macro-blocks)“); MF-Decoder max. 4096 × 2304 | — |
| 33 | 4:4:4 bieten die OS-Kodierer nicht an (C2 Textschärfe) | bestätigt für den Microsoft-Software-MFT, nicht belegbar für Hardware-MFTs und MediaCodec | Ref B1/B3 (Eingabeformate `NV12`, `I420`, `IYUV`, `YUY2`, `YV12`) | MF definiert `eAVEncH264VProfile_444 = 244`; Unterstützung durch Hardware-MFTs nicht geprüft |
| 34 | `tiles`-Schlüsselbild 4096 × 2304 roh 37,7 MB (C2) | bestätigt | 4096 × 2304 × 4 = 37 748 736 Byte | — |
| 35 | iroh 1.3.0 erlaubt Uni-Stream-Grenze je Verbindung (C3 L1, R2) | bestätigt | N1 | — |
| 36 | Endpoint-Konfiguration bleibt unverändert, 0 Uni-Streams für FS/Exec (C3 L1) | bestätigt | `N/share/core/keepalive.rs:9` (`IROH_MAX_CONCURRENT_UNI_STREAMS: u32 = 0`), `:101-118` | — |
| 37 | Datagramme über Relay/TCP ohne Nutzen; FEC wirkt auf der Relay-Strecke nicht; Relay-Verlust erscheint als Verzögerung (C3 L2, FEC-Begründung) | widerlegt (teilweise) | N19: Relay-Server verwirft bei voller Ziel-Warteschlange (512 Pakete/1 MiB); Relay-Client verwirft beim Neuaufbau | Auf der TCP-Teilstrecke wird wiederholt; am Relay entsteht bei Überlast echter Verlust. Die übrigen FEC-Gründe (QUIC wiederholt; Dauerkosten; FEC setzt Datagramme voraus) bleiben unberührt |
| 38 | Sunshine-FEC-Standard 20 % (C3 L2) | bestätigt | Ref C4 (`fec_percentage` Standard 20) | — |
| 39 | Wiederverwendung von `read_payload`/`write_payload` aus `exec_protocol.rs` (Sichtbarkeit `pub(crate)`) (C3 Rahmenformat) | widerlegt (Ist-Zustand) | `N/share/core/exec_protocol.rs:419`, `:433`: `async fn write_payload`/`read_payload` ohne `pub` | Heute privat; Wiederverwendung verlangt eine Sichtbarkeitsänderung in `exec_protocol.rs` |
| 40 | Kopf 5 Byte; `MAX_HANDSHAKE_BYTES` 64 KiB; `MAX_FRAME` 16 MiB (C3 Rahmenformat) | bestätigt | `N/share/core/exec_protocol.rs:12`, `:14`; `N/share/core/framing.rs:11` | — |
| 41 | Staukontrolle bleibt `noq`-Standard, Cubic (C3 Staukontrolle; „für 1.3.0 N1“) | bestätigt | N1 | Datagrammpuffer 1 250 000 / 1 048 576 Byte in 1.3.0 |
| 42 | GCC-Werte: Überlast → 0,85 × Empfangsrate; +8 % je Sekunde; Verlust > 10 % → × (1 − 0,5 p); < 2 % Steigerung (C3) | bestätigt | draft-ietf-rmcat-gcc-02 Zeilen 592 (`eta = 1.08^…`), 637/699 (`beta` 0,85), 733-742 (2–10 % halten, > 10 % senken, < 2 % × 1,05) | Die verlustbasierte Steigerung beträgt laut Entwurf 5 % |
| 43 | Verlustrate aus `Connection::stats()` der letzten Sekunde (C3) | bestätigt mit Hinweis | `iroh-1.3.0/src/endpoint/connection.rs:1062`; `noq-proto-1.3.0/src/connection/stats.rs:236`, `:269` (`lost_packets` je Pfad und Verbindung) | Zähler sind kumulativ; „letzte Sekunde“ = Differenz zweier Abfragen |
| 44 | Iroh wechselt den Pfad selbst (C3 Relay) | bestätigt | `iroh-1.3.0/src/socket.rs:137` (`MAX_MULTIPATH_PATHS = 8`); Ref C1 | Latenzspitzen beim Wechsel: Lücke |
| 45 | ALPN-Fehlschlag → `screen_unsupported` (C3 Erkennung) | bestätigt | N2 | Erkennung über `ConnectionClosed` mit Code `0x178` |
| 46 | Eigener egui/wgpu-Prozess, NV12/I420 → RGB im WGSL-Shader über `egui_wgpu::CallbackTrait`, Upload per `queue.write_texture` (C4 L1) | bestätigt | N3, N20 | 1080p: NV12 1920 × 1080 × 1,5 = 3,11 MB, RGBA 8,29 MB |
| 47 | eframe-Standardrenderer ist wgpu, wenn beide Features aktiv sind (C4 L4) | bestätigt | `eframe-0.29.1/src/epi.rs:528-546` (`#[cfg(feature = "glow")] #[cfg(feature = "wgpu")] return Self::Wgpu`) | — |
| 48 | `AutoVsync` addiert höchstens eine Bildwiederholperiode (C4 Details, R15, Annahme) | widerlegt (durch Standardwerte nicht gedeckt) | N3: Fifo-Warteschlange „approximately 3 frames long“; `desired_maximum_frame_latency` Standard 2 (`wgpu-22.1.0/src/lib.rs:5643`) | Mit Standardwerten bis zu 2 Bilder in der Warteschlange; Abhilfe `desired_maximum_frame_latency = Some(1)` oder `AutoNoVsync` |
| 49 | Android `SurfaceView` + MediaCodec-Decoder, `KEY_LOW_LATENCY` (C4 Android) | bestätigt | `KEY_LOW_LATENCY` API 30 = `minSdk` | — |
| 50 | JNI mit direkten `ByteBuffer`s über `jni` 0.22.4 (C4 Binärpfad) | nicht belegbar | — | Lücke: `jni` 0.22.4 nicht im lokalen Register; Funktionen für direkte Puffer nicht nachgelesen |
| 51 | Windows-Eingabe: `SendInput` mit `MOUSEEVENTF_ABSOLUTE`/`MOUSEEVENTF_VIRTUALDESK`, `KEYEVENTF_SCANCODE`, `KEYEVENTF_UNICODE`; UIPI ohne Fehlermeldung (C5 Windows) | bestätigt | `windows-0.58.0/…/KeyboardAndMouse/mod.rs:395-396`, `:440`; Ref D1 | — |
| 52 | Integritätsprüfung des Vordergrundprozesses über `TokenIntegrityLevel` (C5 Windows Anzeige) | nicht belegbar | N18 | Lücke: Zugriff auf das Token eines erhöhten Prozesses aus Medium IL nicht belegt |
| 53 | Strg+Alt+Entf nur über SYSTEM-Dienst (C5 Windows) | bestätigt mit Ergänzung | `SendSAS`-Referenz (N18) | Zusätzlich Richtlinie „Disable or enable software Secure Attention Sequence“ nötig; alternativ UIAccess-Programm (signiert, `\Program Files\`) |
| 54 | X11-Text über vorübergehende Belegung eines freien Keycodes (C5 Linux X11) | bestätigt | N13 (xdotool, BSD-3-Clause) | — |
| 55 | RemoteDesktop-Portal `NotifyPointerMotionAbsolute`, `NotifyPointerButton`, `NotifyPointerAxis`, `NotifyKeyboardKeycode`, `NotifyKeyboardKeysym`, Touch (C5 Wayland) | bestätigt | Ref D2; `ashpd-0.13.13/src/desktop/remote_desktop.rs:423`, `:573`, `:729` | — |
| 56 | Portal-Backend ohne RemoteDesktop → `control_unavailable` (C5 Wayland) | bestätigt | N11 (`wlr.portal`) | Betrifft alle wlroots-Desktops mit xdpw |
| 57 | Android: `dispatchGesture`, `continueStroke` ab API 26, `performGlobalAction` (`back`, `home`, `recents`, `notifications`, `quick_settings`, `lock_screen`, `power_dialog`), `ACTION_SET_TEXT`, `ACTION_IME_ENTER` (API 30) (C5 Android) | bestätigt | N16: API 24, 26, 16/17/21/28, 30 | — |
| 58 | Windows-Fensterliste: `EnumWindows`, `DWMWA_CLOAKED`, `QueryFullProcessImageNameW` (C5 „Was läuft“) | bestätigt | `windows-0.58.0/…/Dwm/mod.rs` (`DWMWA_CLOAKED`), `…/System/Threading/mod.rs:1504`; Ref D1 | — |
| 59 | X11-Liste über EWMH `_NET_CLIENT_LIST`, `_NET_WM_NAME`, `_NET_WM_PID`, `WM_CLASS`, `_NET_ACTIVE_WINDOW` (C5) | nicht belegbar | — | Lücke: EWMH-Spezifikation nicht abgerufen |
| 60 | Wayland-Liste `ext-foreign-toplevel-list-v1`, sonst `wlr-foreign-toplevel-management` (C5) | bestätigt | N12 (Kompositortabelle) | — |
| 61 | „GNOME (Mutter) und KWin bieten keine Fensterliste für Drittprogramme“ (C5 Wayland) | widerlegt für KWin, bestätigt für Mutter | N12; Ref D2 (GNOME `Introspect` nur mit Allow-Liste) | KWin gibt `org_kde_plasma_window_management` frei, wenn die `.desktop`-Datei sie unter `X-KDE-Wayland-Interfaces` anfordert (Sekundärbeleg KDE-Mailingliste 2024) |
| 62 | systemd-Konvention `app-<id>-<n>.scope`/`app-<id>@<n>.service` (C5 Wayland) | widerlegt (unvollständig) | N12 | Richtig: `app[-<launcher>]-<ApplicationID>-<RANDOM>.scope` und `app[-<launcher>]-<ApplicationID>[@<RANDOM>].service` |
| 63 | Android-Liste mit Bedienungshilfe `getWindows()` (Titel, Paket), sonst `UsageStatsManager` (C5) | bestätigt | `getWindows` API 21, `AccessibilityWindowInfo.getTitle` API 24, `UsageStatsManager.queryEvents` API 21 | `getWindows` braucht das Dienst-Flag `flagRetrieveInteractiveWindows` (Annahme, nicht nachgelesen) |
| 64 | Vorschau 320 × 180 roh 230 KB; Android-Vorschauen über die Bedienungshilfe (C6) | bestätigt | 320 × 180 × 4 = 230 400 Byte; N16 (Abstand ≥ 333 ms) | — |
| 65 | Android-Rückfrage über Benachrichtigung mit „Freigeben“/„Ablehnen“ (C7) | bestätigt mit Bedingung | N15 (Ausnahme „PendingIntent … sent by the system (for example, from a notification tap)“) | Bei Ziel API 35/36 müssen Ersteller bzw. Sender das Hintergrundstartrecht ausdrücklich gewähren |
| 66 | Lizenzen C8: `windows` 0.58 MIT/Apache, `x11rb` 0.13.2 MIT/Apache, `wayland-client` 0.31 MIT, `wayland-protocols` 0.32 MIT, `wayland-protocols-wlr` 0.3 MIT, `ashpd` 0.13 MIT, `lz4_flex` 0.14 MIT, `libloading` 0.8 ISC, `png` 0.18 MIT/Apache, `egui-wgpu` MIT/Apache, `openh264` BSD-2 (C8) | bestätigt | crates.io-API je Version (Abruf 2026-10-17) | — |
| 67 | IronRDP ohne H.264 (C8) | bestätigt | IronRDP-README: Server kodiert „RDP 6.0 bitmap compression, RemoteFX, optional NSCodec, and optional QOI / QOI+zstd“; H.264 nicht genannt | — |
| 68 | RDP auf Client-Windows übernimmt die Konsolensitzung statt sie zu spiegeln (C8, Annahme) | nicht belegbar | N23 (nur Sekundärquellen) | Lücke: Microsoft-Primärquelle |
| 69 | xdotool als GPL-Code (G N13 Quellenhinweis, Grundlage für C5 X11) | widerlegt | N13 | xdotool ist BSD-3-Clause |
| 70 | CI: Xvfb auf Linux-Runner; `tc netem` auf `lo` mit `sudo` (C9) | bestätigt für Xvfb, nicht belegbar für netem | N21 | Lücke: `sch_netem` im Runner-Kernel |
| 71 | CI: Wayland über kopflosen Compositor mit nicht interaktivem Portal-Wähler (C9) | bestätigt für ScreenCast | N11 (xdpw `chooser_type = none`) | Wayland-Steuern nur mit KDE und Permission-Store-Eintrag `kde-authorized`/`remote-desktop` |
| 72 | CI: Windows über die Desktop-Sitzung des Runners (C9, N7) | bestätigt | N7 (WinSta0, `UserInteractive=True`) | Media Foundation fehlt im Bild (R6) |
| 73 | CI: Android im Emulator mit `appops` (C9, N15) | bestätigt | N15 (AOSP `hasProjectionPermission` → `OP_PROJECT_MEDIA`) | — |
| 74 | `smart-explorer --screen-view`, `--screen-consent`, `--screen-indicator` (C4 L1, C7) | widerlegt (Programmname) | N22 | Binary heißt `smart_explorer` (Windows installiert `Smart Explorer.exe`) |
| 75 | OpenH264 als Cisco-Binärdatei über das `openh264`-Crate ladbar (R16, Annahme; C8) | bestätigt | N8 (`features = ["libloading"]`, `OpenH264API::from_blob_path`) | Crate prüft SHA-256 nur für Version 2.6.0 |

---

## 3. Folgen für Entscheidungen

Nur Entscheidungen, deren Grundlage sich durch diese Recherche geändert hat. Die Bewertung trifft der Planverfasser.

1. **C2 Aushandlung („Ein Betrachter, der nur OpenH264 dekodiert, erhält `cb`“) und C2 Dekodierer Linux.** Grundlage widerlegt (Tabelle Nr. 28): OpenH264 dekodiert laut Release-Notizen Constrained High (ohne B-Bilder, progressiv). Die Einschränkung auf `cb` für Linux-Betrachter beruht damit nur noch auf dem README-Wortlaut; widersprüchliche Primärquellen.
2. **C3 L2 (Datagramme + FEC verworfen) und „Vorwärtsfehlerkorrektur: nein“.** Teilgrund „auf der Relay-Strecke wirkt FEC nicht / Verlust erscheint als Verzögerung“ widerlegt (Nr. 37): Das Relay verwirft bei voller Ziel-Warteschlange. Die übrigen Gründe bleiben. Für die Regelung folgt: der Warteschlangenwächter muss unter 1 MiB unbestätigter Daten bleiben.
3. **C1 Windows L4 (`windows-capture` verworfen).** Kostengrund „zweite Bindungsschicht“ widerlegt (Nr. 5): `windows` 0.62.2 ist bereits im Lock. Es bleibt nur „kein Funktionsgewinn“.
4. **C1 Android L2 (Bedienungshilfe als Standbildquelle).** Begründung zu `FLAG_SECURE` widerlegt (Nr. 19); Systemgrenze 333 ms je Aufnahme belegt (N16).
5. **C4 L1 Darstellungstakt (`AutoVsync`, R15).** Annahme widerlegt (Nr. 48): Standardwerte erlauben bis zu 2 wartende Bilder; einstellbar über `desired_maximum_frame_latency` bzw. `AutoNoVsync`; „Mailbox mit Rückfall“ gibt es in wgpu 22 nicht als Modus.
6. **C5 „Was läuft“ unter KDE Wayland.** Grundlage widerlegt (Nr. 61): KWin-Fensterliste ist über `X-KDE-Wayland-Interfaces` in der `.desktop`-Datei erreichbar. Der Rückfall „nur Apps aus systemd-Scopes“ ist für KDE nicht zwingend; die Scope-Namensregel ist zu korrigieren (Nr. 62).
7. **C1 Linux L1 / C8 PipeWire-Lade-ABI.** Mehraufwand belegt (Nr. 12): SPA-POD-Kodierung muss in Rust nachgebaut werden. Die Wahl gegen das `pipewire`-Crate (Nr. 11) bleibt sachlich begründet; die Kosten der eigenen ABI sind höher als im Plan beschrieben.
8. **C1 Linux Persistenz und C5 Wayland-Steuern auf wlroots.** Neue Grundlage (Nr. 10, 56): xdpw bietet kein RemoteDesktop und Persistenz nur mit Nutzervariable `XDPW_PERSIST_MODE`.
9. **C3 Rahmenformat (Wiederverwendung `read_payload`/`write_payload`).** Ist-Zustand widerlegt (Nr. 39): Funktionen sind privat; Änderung an `N/share/core/exec_protocol.rs` (Exec-Modul) wird nötig.
10. **C8 OpenH264 und Release (N25).** Grundlage ergänzt (Nr. 21, 75): Cisco-Bedingung 4 verlangt den vollständigen Lizenztext an der Lizenzstelle der App; das Release verteilt heute keine Crate-Lizenztexte außer Dokany. R16 entfällt; `openh264_abi.rs` ist nicht nötig.
11. **C9 / SCR-M12 Windows-Kodierung in CI (R6).** Grundlage geändert (Nr. 72): Runner-Sitzung ist interaktiv, aber `Server-Media-Foundation` ist nicht installiert und braucht nach Forenberichten einen Neustart.
12. **C4/C7 Programmname der Hilfsfenster.** Nr. 74: `smart_explorer`; auf Linux kann der Daemon `se` sein und findet das GUI-Binary nur als Geschwisterdatei (N22).
13. **C5 Windows Strg+Alt+Entf (TODO SYSTEM-Dienst).** Ergänzt (Nr. 53): zusätzlich Richtlinie „software Secure Attention Sequence“ nötig.
14. **C1 Windows L1 Fabrikwahl.** Präzisiert (Nr. 3): nur die Interop-Fabriken `CreateForMonitor/CreateForWindow` kommen ohne Paketidentität aus; `TryCreateFromDisplayId/WindowId` scheiden aus.
