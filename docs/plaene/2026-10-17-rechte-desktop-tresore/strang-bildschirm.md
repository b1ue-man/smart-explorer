# S4 — Bildschirm-Streaming und Fernsteuerung

Stand: 2026-10-17, Endfassung nach der Nachrecherche (`nachrecherche-bildschirm.md`) und nach den fertigen Strangplänen S1 (`strang-rechte.md`) und S2 (`strang-laufwerke.md`). Lesestand des Codes: `da2646d2`. Teil des Plans `plan.md` (Ergebnis E5). Nichts in dieser Datei ist gebaut oder ausgeführt (AGENTS.md: keine lokalen Builds/Tests).

Kennzeichnung: **Befund** = im genannten Code oder Dokument nachgelesen; **Annahme** = ungeprüfte Grundlage, ausdrücklich so benannt, mit Wirkung und – wo möglich – Abnahmesignal; **Lücke** = genau benannte fehlende Angabe. Abkürzungen: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`, „Ref“ = `docs/refs/bildschirm-streaming-2026-10-17.md`, „Bestand“ = `docs/lesungen/2026-10-17-bildschirm-bestand.md`, „NR“ = `nachrecherche-bildschirm.md` (NR N1–N25 = Antworten, NR-Nr. 1–75 = Revalidierungstabelle Abschnitt 2, NR-Folge 1–14 = Abschnitt 3), „S1-E“ = `strang-rechte.md` Abschnitt E.

## Schleifenstand

**Ziel:** Vom se Desktop (Windows, Linux, Android) zum Bildschirm jedes verbundenen Geräts wechseln, direkt oder innerhalb einer Gruppe (Share-Raum), sehen, was es ausführt (ganzer Bildschirm, einzelnes Fenster oder Liste der laufenden Fenster/Apps) und es bedienen, mit sehr schneller, sehr zuverlässiger Kompression und Dekompression. Quelle: E5 in `plan.md` A.

**Kriterien** (Status für den Planungsteil: „erfüllt“ = Entscheidung belegt und Abnahmesignal definiert; Annahmen sind genannt):

| Kriterium | Herkunft | Nachweis | Status |
|---|---|---|---|
| K1 Wechsel zum Bildschirm jedes verbundenen Geräts, direkt und in Gruppen | E5 „shift to the screen of any other connected device, directly or in the groups“ | C6, SCR-M7, SCR-M9 | erfüllt: Bedienweg festgelegt; Abnahmesignale `screen_task_grid_switch_reuses_connection`, `screen_task_android_grid_switch` |
| K2 Sehen, was das Gerät ausführt (Bildschirm-/Fensterstream oder anderer Weg) | E5 „see aht they are running directly, be it with a window video stream or in other ways“ | C1, C5 | erfüllt mit Annahmen A6 (KWin-Fensterliste) und A7 (systemd-Scopes), beide mit Rückfall und Abnahmesignal; Plattformgrenzen belegt (Wayland-Fensterwahl am Host, GNOME ohne Fensterliste) |
| K3 Bedienen, soweit das OS es erlaubt, ehrliche Anzeige der Grenzen | E5 „operate“ (Folgerung `plan.md` A) | C5, SCR-M6, SCR-M8 | offen in einem Punkt: Wayland-Steuern hat ein definiertes Abnahmesignal (`screen_task_kde_remote_desktop_input`), dessen Ausführbarkeit auf `ubuntu-24.04` unbelegt ist (NR N11 Lücke „KWin kopflos“); übrige Plattformen erfüllt |
| K4 Sehr schnelle, sehr zuverlässige Kompression/Dekompression | E5 | C2, C3, C9 | erfüllt für den Planungsteil: Messgrößen, Messwerkzeug und Schwellen definiert; Schwellen sind Annahme A1 bis zur ersten Remote-Messung |
| K5 Ansehen und Steuern nur mit bewusster Freigabe des Hosts, im Rechtemodell von S1 | AGENTS.md, Sicherheit, S1 | C7, E1 | erfüllt: S1-Vertrag (S1-E) übernommen; Abnahmesignale in SCR-M2 |
| K6 Windows, Linux (X11 und Wayland), Android als Sender und Empfänger, Grenzen belegt | E4 „all suportet systems“ | C1–C5, Plattformprüfungen in D | offen in einem Punkt: Die Media-Foundation-Kette (Windows-Hauptweg der Kodierung) und alle GPU-Hardwarekodierer haben definierte Abnahmesignale, aber auf den GitHub-Runnern keine Ausführungsmöglichkeit (Windows-Bild ohne `Server-Media-Foundation`, keine GPU; NR N7). Offen bis der Hauptagent entscheidet, ob die Suite zusätzlich auf einem Windows-Client-Rechner läuft; ohne diese Entscheidung bleibt der Weg ungeprüft und wird so berichtet |
| K7 Lizenz der Bausteine mit MIT-Verteilung verträglich oder begründet | E7 | C8, SCR-M10 | erfüllt: nur MIT/Apache/BSD/ISC/Zlib-Code im Binary, Pflichten aus NR N25 in SCR-M10; OpenH264 nur als Cisco-Laufzeitdownload; Patentdeckung der OS-Codecs ist Annahme A3 (kein Lizenz-, sondern Patentpunkt, R12) |
| K8 Jede Oberfläche, die den Gegenstand zeigt oder steuert, erhält die Fähigkeit gleichwertig (CLI Text/JSON/Vervollständigung, Desktop-GUI, Android-GUI, Android-Bridge, Daemon-IPC, MCP, Netzprotokoll, Doku) | Auftrag, `docs/oberflaechen/INDEX.md` | D je Meilenstein, `docs/oberflaechen/bildschirm.md` | erfüllt (Ausnahmen mit Grund: Live-Video nicht über MCP; Rückfrageentscheidung nicht über MCP, S1 C7) |
| K9 Bestehendes Verhalten bleibt erhalten | AGENTS.md | C3, `screen_task_fs_exec_unchanged` | erfüllt: eigenes ALPN, Endpoint-Konfiguration unverändert; `exec_protocol.rs` nur Sichtbarkeitsänderung |
| K10 `core/` plattformneutral, Plattformcode unter `os/`, neue Rust-Dateien < 500 Zeilen, plattformspezifische Abhängigkeiten in `target`-Abschnitten | AGENTS.md | Dateilisten in D | erfüllt |
| K11 Keine willkürlichen Grenzen, keine UI-Platzhalter | Auftrag | Grenzliste C, D | erfüllt (jede Grenze mit Protokoll-, Plattform- oder Ressourcengrund genannt) |
| K12 Eine Plattformprüfung je übertragenem Mechanismus | Nutzerregel | Plattformprüfung je Meilenstein | erfüllt |
| K13 Schnittstelle für S5 | Auftrag | E2 | erfüllt |

**Aktiver Stand:** Entwurf C1–C9 in der Endfassung: eigenes ALPN `smart-explorer/share-screen/1`; Steuerstrom plus ein QUIC-Uni-Stream je Bildgruppe; H.264 (`cb`, `high`) über die OS-Codecs, Cisco-OpenH264 2.6.0 als nachladbarer Software-Weg, eigener verlustfreier Kachel-Codec; Aufnahme WGC über die Interop-Fabriken (DXGI-Rückfall), Wayland-Portal + PipeWire (zur Laufzeit geladen, SPA-POD in Rust), X11 MIT-SHM, Android MediaProjection und Bedienungshilfe-Standbilder; Eingabe SendInput, XTest, RemoteDesktop-Portal (GNOME/KDE), Android-Bedienungshilfe; Hostprozess für Aufnahme/Kodierung; Betrachter als eigener egui/wgpu-Prozess (`AutoVsync`, `desired_maximum_frame_latency = 1`) bzw. Android-SurfaceView; Rechte, Rückfrage und Protokoll über S1 (`access_grant`, `access_approval`, `access_audit`); VA-API nicht in diesem Stapel.

**Nächster Schritt:** Entscheidung des Hauptagenten zu K6 (Windows-Client-Stufe der Suite ja/nein); danach Umsetzung in der Reihenfolge D. Rückkehrpunkt: Abschnitt D.

**Schleifenstatus:** offen. Genaue Gründe: (1) K6 — Media-Foundation-Kette und GPU-Hardwarekodierer ohne ausführbares Abnahmesignal auf den verfügbaren Runnern (NR N7); (2) K3 — Wayland-Steuern-Abnahmesignal hängt an der unbelegten Startbarkeit von KWin kopflos samt `xdg-desktop-portal-kde` auf `ubuntu-24.04` (NR N11). Alle übrigen Kriterien sind für den Planungsteil erfüllt.

## B — Bestand dieses Strangs (nur Entscheidungsrelevantes)

Quellen: Bestand (F1–F7, Abschnitte 3–5), Ref (A–E), NR (N1–N25), eigene Lektüre am 2026-10-17.

**Transport**
- Befund: `iroh` 1.3.0 mit `noq` 1.3.0 (Bestand F1). Staukontrolle Cubic, Datagrammpuffer 1 250 000 / 1 048 576 Byte, von iroh nicht überschrieben (NR N1).
- Befund: Endpoint-ALPNs `smart-explorer/share-fs/3`, `smart-explorer/share-exec/2`, `smart-explorer/paired-link/1` (`N/share/core/node.rs:152-156`); Verteilung `N/share/core/node_accept.rs:137-147`.
- Befund: endpunktweite Transportkonfiguration mit 64 bidirektionalen und 0 unidirektionalen Streams (`N/share/core/keepalive.rs:9`, `:101-118`).
- Befund (NR N1): `Connection::set_max_concurrent_uni_streams`, `open_uni`, `accept_uni` je Verbindung in iroh 1.3.0 (`iroh-1.3.0/src/endpoint/connection.rs:885`, `:904`, `:1124-1131`). Nach `SendStream::reset` gibt der Empfänger den Verbindungskredit sofort frei; der Stream-Zählplatz wird erst frei, wenn der Empfänger den Stream stoppt oder den Reset liest und den Stream freigibt.
- Befund (NR N2): Eine Gegenstelle ohne das ALPN bricht den Handshake mit TLS-Alarm 120 ab; beim Verbindenden `ConnectionError::ConnectionClosed` mit Transportcode `0x178`.
- Befund (NR-Nr. 39): `read_payload`/`write_payload` in `N/share/core/exec_protocol.rs:419`, `:433` sind heute **privat**.
- Befund (NR N19): Der Relay-Server verwirft Pakete, wenn die Ziel-Warteschlange voll ist (512 Pakete oder 1 MiB je Ziel; `V/iroh-relay-1.0.0/src/server/client.rs:242-258`, `iroh-relay-1.3.0/src/server/clients.rs:212-224`). Auf der TCP-Teilstrecke wird wiederholt, am Relay entsteht bei Überlast echter Verlust.

**Rechte (S1, fertig)**
- Befund (S1-E): `N/share/core/access_grant.rs` mit `Capability { ExecCommands, ExecTerminal, ScreenView, ScreenControl }`, `CapabilityDecision { Off, Ask, Allow }`, `CapabilityState`; `N/share/core/relation_proof.rs::prove_relation`; `N/share/core/access_authorize.rs::capability_for(…, now)`; `N/share/core/access_catalog.rs::capability_catalog(&HostFacts)`; `RestrictionReason::CapabilityNarrowed`, `GrantExpired`; `N/share/os/shared/access_audit.rs::record_access`; Rückfragedienst `N/share/core/access_approval.rs` mit Fenster `smart_explorer --access-consent <ID>` (S1 PRM-M1, PRM-M3, PRM-M4).

**Prozesse, Programmnamen, Sitzung**
- Befund (NR N22): Das GUI-Binary heißt `smart_explorer` (Paketname `native/Cargo.toml:2`), installiert unter Windows als `Smart Explorer.exe` in `%LOCALAPPDATA%\Programs\Smart Explorer` (`native/installer.nsi:47`, `:60`), unter Linux als `$HOME/.local/opt/smart-explorer/smart_explorer` mit Verweis `~/.local/bin/smart_explorer` (`install-linux.sh:8-15`, `:369`). `se` findet es als Geschwisterdatei (`N/updater/os/shared/terminal.rs:63-80`); fehlt es, ist die Installation `TerminalOnly`.
- Befund (NR N22): Windows-Autostart startet immer das GUI-Binary mit `--sync-daemon` (`N/autostart/os/windows.rs:12-37`); Linux-Autostart startet das aufrufende Programm selbst, also `se` oder `smart_explorer` (`N/autostart/os/linux_os.rs:56-61`). Der Daemon läuft damit in der grafischen Sitzung des Nutzers; ein Linux-root-Worker hat keine.
- Befund: `install-linux.sh:389-403` schreibt `smart-explorer.desktop` mit `Exec=$APP_BIN` (absoluter Pfad).
- Befund: Präzedenzfälle Prozessisolation (`N/daemon/os/shared/mount_host_process.rs`), Laufzeitdownload mit Hash und `https_only(true)` (`N/mount/os/windows/runtime_install_download.rs:75-95`), Byte-Stream über die Daemon-IPC (`IpcRequest::ExecStream`, `N/daemon/os/shared/ipc_protocol.rs:299-303`).
- Befund (NR N23): Smart Explorer nennt keine allgemeine Windows-Mindestversion; Rust-Ziel ab Windows 10 → faktisch Windows 10 ab 1507; WGC-Interop ab 1903.

**Darstellung**
- Befund: eframe 0.29.1 mit `glow` und `wgpu`; Standard ist dann `Renderer::Wgpu` (`eframe-0.29.1/src/epi.rs:528-546`, NR-Nr. 47).
- Befund (NR N3): `WgpuConfiguration { present_mode = AutoVsync, desired_maximum_frame_latency = None }` als Standard, wgpu-Standard für die Bildlatenz 2; `Mailbox` stürzt ab, wenn nicht unterstützt; `AutoNoVsync` = Immediate → Mailbox → Fifo.
- Befund (NR N4): `egui::Key` hat keine Super/Windows-, Menü-, Druck-, Rollen-, Pause-, Feststell-, NumLock- und Medientasten; Nummernblock wird auf die Haupttasten abgebildet; IME über `Event::Ime` nur, solange `PlatformOutput::ime` gesetzt ist.
- Befund (NR N20): `R8Unorm`/`Rg8Unorm` und `write_texture` funktionieren auch auf dem GL-Rückfall von wgpu 22.

**Android**
- Befund: JSON-Brücke ohne Binärpfad (Bestand F6), `jni = "=0.22.4"`; `minSdk = 30`, `targetSdk = 36` (`android/app/build.gradle.kts:84-90`); Sideload-Verteilung (`README.md:1057`, `docs/RELEASING.md:751`); `AboutScreen.kt` ist die Lizenzstelle der App (`A/ui/settings/AboutScreen.kt:84`, `:98`).
- Befund (NR N15, N16): alle benötigten MediaFormat-Schlüssel ≤ API 30; `appops … PROJECT_MEDIA allow` überspringt den Projektionsdialog; `takeScreenshot` mindestens 333 ms Abstand; Zustimmungsaktivität nur aus `PendingIntent` einer Benachrichtigung startbar, bei Ziel 35/36 mit ausdrücklich gewährtem Hintergrundstart.

**Abhängigkeiten und Lizenzen**
- Befund: im Lock bereits vorhanden: `windows` 0.58 (direkt) und 0.62.2 (transitiv über `netwatch`, `pageant`, `wmi`; NR-Nr. 5), `zbus` 5.16 (direkt), `x11rb` 0.13.2, `wayland-client` 0.31.14, `wayland-protocols` 0.32.12, `wayland-protocols-wlr` 0.3.12, `wayland-protocols-plasma` 0.3.12, `lz4_flex` 0.14.0, `libloading` 0.8.9, `png` 0.18.1, `sha2` 0.10.9, `ureq` 2.12.1, `serde_repr` 0.1.20, `enumflags2` 0.7.12. Nicht im Lock: `ashpd` 0.13.13, `openh264` 0.9.9, `openh264-sys2` 0.9.9, `wide`, `bzip2` 0.6.1, `libbz2-rs-sys`.
- Befund (NR N25): Das Release verteilt Lizenztexte nur für Dokany (`native/installer.nsi:134-139`); kein Mechanismus für Crate-Lizenzen; Desktop-GUI hat keine Lizenzansicht.

**Prüfinfrastruktur**
- Befund (NR N7, N21): GitHub-Windows-Runner (`windows-2022`, `windows-2025`) laufen in einer interaktiven Sitzung (`WinSta0`), ohne `Server-Media-Foundation`; `ubuntu-24.04` hat `xvfb` und `iproute2`, kein PipeWire. Muster für Gerätepaar Emulator ↔ Runner: `native/review-task-device.py:61-179`.

## C — Entscheidungen

Bewertungszeichen: „+“ erfüllt, „○“ teilweise, „−“ verfehlt. Korrekturen aus der Nachrecherche sind mit „(korrigiert, NR-Nr. x)“ markiert.

### C1 — Aufnahme je Plattform

**Windows**

| Lösung | K2 | K4 | K6 | K7/K10 | Bewertung |
|---|---|---|---|---|---|
| L1 Windows Graphics Capture über `windows` 0.58, Elemente aus `IGraphicsCaptureItemInterop::CreateForMonitor`/`CreateForWindow`, Rahmenpool `CreateFreeThreaded` | + Bildschirm und Fenster | + D3D11-Textur bis zum Kodierer | ○ ab Windows 10 1903, Cursor-Schalter ab 2004 | + nur Features des vorhandenen Crates | gewählt |
| L2 DXGI Desktop Duplication | ○ nur ganze Ausgänge | + | + ab Windows 8 | + | gewählt als Rückfall für Bildschirmaufnahme (Windows 10 1507–1809, darunter LTSC 2015/2016/2019; NR N23) |
| L3 GDI `BitBlt`/`PrintWindow` | ○ | − CPU-Kopie je Bild | + | + | verworfen |
| L4 `windows-capture` 2.0.1 (MIT) | + | + | ○ | ○ | verworfen (korrigiert, NR-Nr. 5): Das Crate nutzt `windows` 0.62.2, das bereits im Lock ist – eine neue Bindungsschicht entsteht nicht. Verbleibender Grund: kein Funktionsgewinn, und seine Texturen sind 0.62-Typen, die zur Media-Foundation- und D3D11-Kette in 0.58-Typen nur über rohe COM-Zeiger passen |

Wahl im Einzelnen:
- Nur die Interop-Fabriken `CreateForMonitor`/`CreateForWindow`; `GraphicsCaptureItem::TryCreateFromDisplayId`/`TryCreateFromWindowId` werden nicht verwendet, weil sie `RequestAccessAsync(Programmatic)` mit Paket-Capability verlangen (NR N24, NR-Nr. 3). Annahme A8: Windows 11 24H2/25H2 verhält sich wie dokumentiert (keine Zustimmung für die Interop-Fabriken). Abnahmesignal `screen_task_windows_wgc_snapshot_matches` (Runner `windows-2025`, Build 26100).
- Der gelbe WGC-Rahmen bleibt; se kann ihn ohne Paket-Capability `graphicsCaptureWithoutBorder` gar nicht abschalten (NR-Nr. 7).
- Minimierte Fenster: Zustand `window_minimized` über `IsIconic(hwnd)` je Aufnahmetakt, nicht über ausbleibende Bilder (korrigiert, NR-Nr. 6: das Ausbleiben ist nur durch einen WebRTC-Kommentar belegt). Abnahmesignal `screen_task_windows_minimized_state`.
- Sicherer Desktop (UAC-Abfrage, Sperrbildschirm): Für DXGI belegt nicht lesbar (Ref A2); für WGC undokumentiert (NR-Nr. 8) → Annahme A9: WGC liefert dort keine verwertbaren Bilder. Erkennung unabhängig vom Aufnahmeweg: `OpenInputDesktop(0, false, DESKTOP_READOBJECTS)` scheitert oder `GetUserObjectInformationW(UOI_NAME)` ≠ `Default` → Zustand `secure_desktop`, Aufnahme pausiert. Abnahmesignal `screen_task_windows_lock_reports_secure_desktop` (`LockWorkStation` als letzter Schritt des Windows-Jobs).
- Mehrere Monitore: je Sitzung ein Monitor oder ein Fenster (Grund: Aufnahme-Elemente je Ausgang; zwei 4K-Monitore ergeben 64 800 > 36 864 Makroblöcke, NR-Nr. 9).
- Ein SYSTEM-Hilfsdienst für den sicheren Desktop ist nicht Teil dieses Strangs; TODO-Eintrag mit den belegten Bedingungen (NR N18: Dienst oder UIAccess, Richtlinie „Disable or enable software Secure Attention Sequence“ für Strg+Alt+Entf).

**Linux**

| Lösung | K2 | K5 | K6 | K10 | Bewertung |
|---|---|---|---|---|---|
| L1 Portal ScreenCast + PipeWire, PipeWire zur Laufzeit über `libloading` (eigene ABI-Teilmenge, SPA-POD in Rust) | + Monitor und Fenster; Fenster wählt der Host-Nutzer | + Systemdialog, `persist_mode` 2 | ○ GNOME und KDE mit Persistenz; wlroots (xdpw) nur mit Nutzervariable `XDPW_PERSIST_MODE` (NR N11) | + keine Bauabhängigkeit, keine harte Laufzeitbindung | gewählt für Wayland |
| L2 Crate `pipewire` 0.10.1 | + | + | + | − bindgen/`system-deps` beim Bau, dynamische Bindung (NR-Nr. 11) | verworfen |
| L3 X11 MIT-SHM (`attach_fd`), DAMAGE, XFixes-Cursor, RandR, Composite über `x11rb` 0.13.2 | + | ○ X11 fragt nicht → se-Anzeigefenster (C7) | + | + | gewählt für X11 |
| L4 GStreamer | + | + | + | − LGPL-Stack | verworfen |
| L5 KMS/DRM | ○ | − umgeht die Portal-Zustimmung | ○ | − | verworfen; tragender Grund ist K5 (die Rechte-Angabe `CAP_SYS_ADMIN` ist Annahme ohne Wirkung auf die Entscheidung, NR-Nr. 14) |
| L6 `wlr-screencopy` | ○ | − kein Dialog | − nur wlroots | ○ | verworfen |

Wahl im Einzelnen:
- Sitzungserkennung: `WAYLAND_DISPLAY` gesetzt oder `XDG_SESSION_TYPE=wayland` → L1; sonst `DISPLAY` → L3; sonst `no_display`.
- Portal über `ashpd` 0.13.13 mit `default-features = false, features = ["tokio", "screencast", "remote_desktop"]` (Feature `pipewire` bleibt aus); verträglich mit `zbus` 5.16 (NR N9). `ashpd::register_host_app("smart-explorer")` wird vor dem ersten Portalaufruf versucht; Fehler (Portal vor 1.19.4, abgelehnte App-ID ohne Punkt) werden ignoriert, weil Persistenz die Registrierung laut Primärquellen nicht verlangt (NR N9, Annahme A10 mit Abnahmesignal `screen_task_wayland_token_replaced_after_start`).
- PipeWire-Teilmenge (NR N10): Bibliothek `libpipewire-0.3.so.0`; `pw_stream_events` Version 2 mit festgelegter Feldreihenfolge; der SPA-POD-Builder ist `static inline` und nicht exportiert → POD-Kodierung für `EnumFormat` (Video roh, Shared Memory, BGRx/RGBx, Größen- und Bildratenbereich) wird in Rust nachgebaut (`spa_pod.rs`). Layout von `pw_buffer`/`spa_buffer` je Version ist Annahme A11 (stabile 0.3-ABI); Absicherung durch Prozessisolation und Abnahmesignal `screen_task_wayland_portal_bench` auf der im Runner installierten PipeWire-Version.
- Shared-Memory-Puffer, kein DMA-BUF (Grund: Kodierer ist OpenH264 auf der CPU; VA-API nicht in diesem Stapel, C2).
- Cursor: Portal `cursor_mode` Embedded (GNOME, KDE, xdpw bieten ihn an; NR-Nr. 16); X11: XFixes-Cursor auf der CPU eingezeichnet.
- `restore_token` je Hostnutzer und Sitzungsart in `screen-portal-tokens.json` (eigentümergeschützt, `support_dirs::app_data_dir()`), nach jedem `Start` ersetzt; „Freigabe vergessen“ löscht. Unter wlroots zeigt se den Hinweis „Dauerhafte Freigabe verlangt `XDPW_PERSIST_MODE=permanent` in der Umgebung des Portal-Dienstes“, weil se diese Variable nicht setzen kann (NR N11).

**Android**

| Lösung | K2 | K5 | K6 | Bewertung |
|---|---|---|---|---|
| L1 MediaProjection → VirtualDisplay → `MediaCodec.createInputSurface`, Vordergrunddienst Typ `mediaProjection` | + Bildschirm; App ab 14 QPR2 durch Wahl am Telefon | + Systemdialog je Sitzung | + | gewählt für Video |
| L2 Bedienungshilfe `takeScreenshot` (API 30) als Standbildquelle | ○ Standbilder, höchstens 3 je Sekunde (Mindestabstand 333 ms, NR N16) | + Nutzer aktiviert die Bedienungshilfe einmal; se-Rechte und se-Anzeige gelten | + ab API 30 | gewählt für Vorschauen und Standbildmodus (korrigiert, NR-Nr. 19: `takeScreenshot` der ganzen Anzeige liefert auch bei sicheren Fenstern ein Bild; Annahme A12: `FLAG_SECURE`-Inhalte erscheinen darin schwarz; Abnahmesignal `screen_task_android_still_secure_window_black`) |
| L3 Shizuku-`UserService` als Shell-Prozess (scrcpy-Muster, Binder-Datenweg) | + | ○ einmalige Einrichtung, ohne root nach jedem Neustart | ○ versteckte APIs | nicht in diesem Stapel; TODO mit NR N17 (Binder statt Abstract-Socket, `screenrecord` mit 180-s-Grenze und `--time-limit 0`) |

Ablauf L1 (NR N15): Zustimmung (`createScreenCaptureIntent`) vor `startForeground` mit Typ `FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION`; Projektion endet beim Sperren → Zustand `host_locked`.

Quellen C1: Ref A1–A5; NR N5, N9–N11, N15–N17, N23, N24; Bestand F5–F7.

### C2 — Kodierung und Dekodierung

| Lösung | K4 | K6 | K7 | K10 | Bewertung |
|---|---|---|---|---|---|
| L1 H.264 über OS-Codecs (Media Foundation, MediaCodec), Software-Weg OpenH264 2.6.0 als Cisco-Datei, vom Nutzer nachgeladen | + Hardware auf Windows und Android, ausgereifte Niedriglatenz-Schalter | + | ○ OS-Codecs: Patentdeckung Annahme A3; Cisco-Datei: Patentdeckung unter den Bedingungen der Binärlizenz (NR N8) | + kein C-Code im eigenen Build (`openh264` mit Feature `libloading`) | gewählt |
| L2 eigener verlustfreier Kachel-Codec `tiles` (64×64, `lz4_flex`) | ○ ideal für Standbilder/Text | + überall | + | + | gewählt als zweiter Codec |
| L3 FFmpeg | + | + | − LGPL/GPL | − | verworfen |
| L4 x264 im Produkt | + | + | − GPL-2 | − | verworfen (in CI nur als Prüfwerkzeug zur Erzeugung einer Testdatei, nicht verteilt; SCR-M3) |
| L5 AV1 als Hauptcodec | ○ | ○ | + | − C/ASM-Toolchain | nicht in diesem Stapel; Codec-Kennung im Protokoll |
| L6 HEVC | + | ○ Windows-Dekodierung braucht nach Ref eine Store-Erweiterung (Annahme ohne Wirkung, NR-Nr. 23: tragender Grund ist, dass HEVC für diesen Stapel keinen Bedarf deckt, den H.264 nicht deckt) | ○ | + | verworfen |
| L7 OpenH264 selbst gebaut und gebündelt | ○ | + | − keine Patentdeckung | ○ | verworfen |
| L8 VA-API (Linux-Hardware) | + | ○ Fedora-Mesa ohne H.264-Kodierung (NR N14) | + libva MIT | − ABI-Teilmenge, Einstiegspunkte je GPU-Generation und DMA-BUF-Import unbelegt (NR N14) | nicht in diesem Stapel (korrigiert: bisher bedingter Meilenstein SCR-M10; Go-Kriterium nicht erfüllbar, NR-Folge zu N14) → TODO `SCREEN-VAAPI` |

**Kodierer (Reihenfolge der Wahl):**
- Windows: Hardware-MFT (`MFTEnumEx` mit `MFT_ENUM_FLAG_HARDWARE`, asynchrones Modell) → Software-MFT `Mfh264enc.dll` → OpenH264, wenn nachgeladen → `tiles`. Verfügbarkeit von Media Foundation wird zur Laufzeit ermittelt (`MFStartup` und `MFTEnumEx` liefern keinen H.264-Kodierer → kein MF-Eintrag), nicht aus der Edition abgeleitet (korrigiert, NR-Nr. 25: dass N-Editionen kein MF haben, ist nicht belegt). Abnahmesignal `screen_task_codec_inventory_without_mf` (GitHub-Windows-Runner ohne `Server-Media-Foundation`).
- Linux: OpenH264, wenn nachgeladen → `tiles`.
- Android: MediaCodec-Hardware-Kodierer (`isHardwareAccelerated`, API 29) → Software-Kodierer des Systems → `tiles` aus Bedienungshilfe-Standbildern.

**Dekodierer:**
- Windows: MF-H.264-Decoder (Baseline/Main/High bis Level 5.1, höchstens 4096 × 2304; DXVA für Constrained Baseline nur mit `constraint_set1_flag = 1`, das Constrained Baseline erfüllt; NR N6).
- Linux: OpenH264-Decoder. Korrigiert (NR-Nr. 28): Die Release-Notizen (v1.5.0 „Constrained High“, v2.0.0 B-Bilder Main/High) widersprechen dem README („Constrained Baseline“). Gewählt: OpenH264-Betrachter melden `cb` und `high`; Grund: der Host erzeugt `high` ohnehin progressiv und ohne B-Bilder (= Constrained High). Annahme A4 mit Abnahmesignal `screen_task_openh264_decodes_constrained_high`; schlägt es fehl, meldet `N/screen/os/shared/codec_inventory.rs` für OpenH264 nur `cb` (eine Zeile, Entscheidungsregel festgelegt).
- Android: MediaCodec direkt auf die Fläche.

**Aushandlung:** Host meldet `{codec, profile, hardware, max_width, max_height}`, Betrachter `{codec, profile, hardware}`; Rangfolge Hardware/Hardware → Hardware/Software → Software/Hardware → Software/Software, in gleicher Stufe `high` vor `cb`, `tiles` immer als letzter Eintrag; Betrachter kann erzwingen (`--codec`, Menü „Kodierung“).

**Niedriglatenz-Einstellungen:** keine B-Bilder (MF `CODECAPI_AVEncMPVDefaultBPictureCount = 0`; MediaCodec `KEY_MAX_B_FRAMES = 0`, API 29); MF `CODECAPI_AVLowLatencyMode = TRUE` (Decoder `VT_UI4`); Profil `cb` = MF `eAVEncH264VProfile_ConstrainedBase` (256), lehnt ein Hardware-MFT es ab → Software-MFT für `cb` bzw. `high`, wenn der Betrachter `high` dekodiert (NR N6; Annahme A5: Laufzeitänderung von Bitrate und Schlüsselbildanforderung wirkt bei allen zertifizierten Hardware-MFTs – Abnahmesignal nur auf einem Rechner mit MF und GPU, siehe K6); MediaCodec `KEY_PRIORITY = 0`, `KEY_LATENCY = 1`, Decoder `KEY_LOW_LATENCY = 1` (beide werden ignoriert, wenn der Codec sie nicht kann; R7); CBR, wo angeboten, sonst VBR mit Spitze = Ziel; unendliche Bildgruppe, Schlüsselbild auf Anforderung, beim Start, bei Größenwechsel; Bitrate zur Laufzeit (MF `CODECAPI_AVEncCommonMeanBitRate`, MediaCodec `PARAMETER_KEY_VIDEO_BITRATE`, OpenH264 `ENCODER_OPTION_BITRATE`). Bitstrom Annex B, eine Zugriffseinheit je Bild, SPS/PPS vor jedem IDR.

**OpenH264-Bezug (NR N8):** Crate `openh264` 0.9.9 mit `default-features = false, features = ["libloading"]` (kein C-Code; zieht `openh264-sys2` 0.9.9, `wide`, `libloading`, `sha2`); Laden über `OpenH264API::from_blob_path` (prüft SHA-256 der entpackten Datei gegen die Crate-Liste, nur Version 2.6.0). Download per HTTPS (`ureq`, `https_only(true)`) von `https://ciscobinary.openh264.org/openh264-2.6.0-win64.dll.bz2` bzw. `…/libopenh264-2.6.0-linux64.8.so.bz2`; eigene Pins in `openh264_pins.rs`: `.bz2` Windows `dab5f2a872777f9a58b69bfa9fbcf20d9f82f2d6ec91383fd70bff49bd34ac9f`, entpackt `2076cb5675ec6c1a4c70e7a2a322552f547b6eeed649d6dfcd9e02a543b24691`; `.bz2` Linux `27ab53323c110b76214c1c72222f459d17febbcd1e252136cadc292b0308d75b`, entpackt `2f0cde7c6a6abcf5cae76942894ea42897fa677bce4ed6c91a24dd1b041d5f04`. Entpacken mit `bzip2` 0.6.1 (reines Rust über `libbz2-rs-sys`). Cisco veröffentlicht nur MD5; Integrität kommt allein aus den SHA-256-Pins. 2.6.0 enthält den Decoder-Fix aus 2.5.1 (Commit `63db555e`). Die Datei `openh264_abi.rs` entfällt.

**Grenzen mit Grund:** größte Ausgabe 4096 × 2304 (36 864 Makroblöcke, H.264-Level-5.1/5.2, OpenH264 und MF-Decoder; NR-Nr. 32); kleinste Zielbitrate 150 kbit/s (Annahme A1, kalibriert über `se screen bench`).

**Textschärfe:** 4:4:4 nicht verwendet (Software-MFT, OpenH264 und MediaCodec bieten es nicht; Hardware-MFTs ungeprüft, NR-Nr. 33). Stattdessen: keine Verkleinerung bei 1:1-Anzeige; „Nachschärfen“ (100 ms nach der letzten Änderung 1 s lang Wiederholbilder mit 10 Bildern/s; Annahme A2, Messgröße `static_psnr_db`); Schalter „Schärfe vor Flüssigkeit“ (≤ 15 Bilder/s, volle Auflösung); Modus „Verlustfrei“ (`tiles`).

**Kachel-Codec `tiles`:** 64×64-Kacheln mit 64-Bit-Hash, gesendet nur geänderte Kacheln `{x, y, w, h, lz4(BGRA)}`; Schlüsselbild = alle Kacheln; Datensätze ≤ 4 MiB mit Fortsetzungsflag (ein verlustfreies 4096 × 2304-Bild hat roh 37 748 736 Byte).

Quellen C2: Ref B1–B4; NR N6, N8, N14, N15; NR-Nr. 23–34.

### C3 — Transport über die bestehende Iroh-Verbindung

| Lösung | K4 | K9 | Relay | Bewertung |
|---|---|---|---|---|
| L1 ALPN `smart-explorer/share-screen/1`; bidirektionaler Steuerstrom (Betrachter öffnet); je Bildgruppe ein Uni-Stream vom Host; Betrachter erlaubt Uni-Streams nur auf dieser Verbindung (`set_max_concurrent_uni_streams(4)`) und stoppt zurückgesetzte Streams sofort | + | + Endpoint unverändert | + | gewählt |
| L2 QUIC-Datagramme + FEC + NACK | + | + | ○ | verworfen (Gründe unten, korrigiert) |
| L3 ein Stream für alles | ○ Kopfblockade | + | + | verworfen |
| L4 Fähigkeit im FS-ALPN | − teilt Fenster mit Übertragungen | − | + | verworfen |
| L5 Merkmal im Exec-ALPN | ○ | − koppelt Rechte | + | verworfen |
| L6 WebRTC | + | − zweiter Stack | ○ | verworfen |

**Rahmenformat:**
- Steuerstrom: Kopf 5 Byte (Tag u8, Länge u32 BE). Die bisher **privaten** Funktionen `read_payload`/`write_payload` (`N/share/core/exec_protocol.rs:419`, `:433`) werden in SCR-M1 `pub(crate)` (korrigiert, NR-Nr. 39; reine Sichtbarkeitsänderung im Exec-Modul, Verhalten gleich; Zusammenführung mit S1 PRM-M2, das dieselbe Datei um Rahmen 39 erweitert).
- Tags: 1 `SERVER_HELLO`, 2 `CLIENT_HELLO`, 3 `HELLO_OK`, 4 `HELLO_ERROR`, 5 `APPROVAL_PENDING`; Betrachter→Host 16 `SET_MODE`, 17 `INPUT`, 18 `FRAME_ACK`, 19 `KEYFRAME_REQUEST`, 20 `PING`, 21 `LIST_WINDOWS`, 22 `SNAPSHOT`, 23 `CONTROL_REQUEST`, 24 `PREVIEW_ACK`; Host→Betrachter 32 `STATE`, 33 `PONG`, 34 `WINDOWS`, 35 frei (Standbilder laufen als Uni-Stream der Art `snapshot`), 36 `STATS`, 37 `CONTROL_STATE`, 38 `ERROR`. JSON-Rahmen ≤ 64 KiB (wie `MAX_HANDSHAKE_BYTES`), `WINDOWS` ≤ 1 MiB (4096 Einträge × 256 Byte).
- Bildgruppen-Stream: Kopf `SCRV`, Version, `gop_id` u64, Art (`video`, `preview`, `snapshot`), Codec, Profil, Breite, Höhe; Bilder `{len u32, frame_id u64, capture_ts_us u64, flags u8, payload}`, ein Bild ≤ 16 MiB (`MAX_FRAME`, `N/share/core/framing.rs:11`).
- Eingabe-Batch: `PointerAbs{x u16, y u16}` (0–65535 auf die Quelle), `Button`, `Wheel` (1/120 Rasten), `Key{hid_usage u16, down}`, `Text{utf8}`, `Touch{id, phase, x, y}`, `Action{code}` mit `back`, `home`, `recents`, `notifications`, `quick_settings`, `lock_screen`, `power_dialog`.

**Uni-Stream-Zählplätze (neu, NR N1):** Der Betrachter ruft nach dem Lesen eines Reset-Fehlers `stop` auf bzw. gibt den Stream frei; sonst bliebe der Zählplatz belegt und die Grenze 4 wäre nach vier Resets erschöpft. Grund für 4: höchstens eine laufende und eine neue Bildgruppe je Art (`video`, `preview`) gleichzeitig. Abnahmesignal `screen_task_reset_streams_free_slots`.

**Staukontrolle und Bitrate:** QUIC-Cubic bleibt (NR N1). Darüber eine Regelung nach GCC (Ref C2, NR-Nr. 42): (1) verzögerungsbasiert aus `FRAME_ACK` (Steigung der Laufzeitdifferenz der letzten 20 Bilder); Überlast → Ziel = 0,85 × Empfangsrate; sonst × 1,08 je Sekunde; (2) verlustbasiert aus der Differenz zweier `Connection::stats()`-Abfragen im Sekundenabstand (Zähler sind kumulativ, NR-Nr. 43): > 10 % → Ziel × (1 − 0,5 p); 2–10 % halten; < 2 % → × 1,05 (korrigiert, NR-Nr. 42); (3) Warteschlangenwächter: unbestätigte Bytes > Grenze → nächstes Aufnahmebild wird nicht kodiert (Verwerfen nur vor dem Kodieren); Grenze = Zielrate × 100 ms, auf dem Relay-Pfad höchstens 768 KiB (neu, NR N19: das Relay verwirft ab 1 MiB je Ziel; 256 KiB Abstand für Steuerstrom und gleichzeitige Verbindungen zum selben Gerät); (4) älteste unbestätigte Daten > 1 s → Bildgruppen-Stream zurücksetzen, IDR auf neuem Stream. Abstufung: zuerst Bildrate (bis 10/s), dann Auflösung × 0,75. Startwerte direkt 6 Mbit/s, Relay 2 Mbit/s (Annahme A1). Pfadart aus `transport_label`.

**Schlüsselbild:** `KEYFRAME_REQUEST` bei Dekodierfehler, Beitritt und nach Reset; höchstens eine offene Anforderung (ein IDR kostet ein Vielfaches eines P-Bildes). Kein Intra-Refresh, keine Referenzinvalidierung.

**Vorwärtsfehlerkorrektur: nein.** Gründe (korrigiert, NR-Nr. 37): QUIC wiederholt verlorene Pakete; FEC kostet dauerhaft Bitrate (Sunshine-Standard 20 %); FEC setzt Datagramme und eigene Paketierung voraus. Gestrichen ist der frühere Grund „Relay-Verlust erscheint nur als Verzögerung“: das Relay verwirft bei voller Ziel-Warteschlange; dagegen wirkt der Warteschlangenwächter (3), und FEC-Mehrbitrate würde den Überlauf dort verschärfen. Wiedervorlage, wenn `screen_task_netem_recovery` bei 2 % Verlust p95-Latenz über dem Ziel zeigt.

**Ältere Gegenstellen:** `ConnectionClosed` mit Transportcode `0x178` während des Verbindungsaufbaus → `screen_unsupported` „Gerät kennt die Bildschirmfreigabe nicht (ältere Version)“; jeder andere Fehler bleibt Verbindungsfehler (NR N2). Annahme ohne Wirkung: gleiches Verhalten über das Relay (Abbruch im TLS-Handshake Ende-zu-Ende).

Quellen C3: Ref C1–C4; NR N1, N2, N19; NR-Nr. 35–45; `N/share/core/keepalive.rs`.

### C4 — Darstellung

**Desktop:** L1 eigener Betrachterprozess `smart_explorer --screen-view …` (Windows `Smart Explorer.exe`; korrigiert, NR-Nr. 74) mit eframe/wgpu wie die App; NV12/I420 → RGB im WGSL-Shader über `egui_wgpu::CallbackTrait`, Ebenen als `R8Unorm`/`Rg8Unorm` per `queue.write_texture` (gilt auch auf GL, NR N20); 1080p: 3,11 MB statt 8,29 MB je Bild. Verworfen: Viewport im Hauptprozess (neuer Zustand im Haupt-`App`, CLI bräuchte die laufende App), reine CPU-Umrechnung (nur für Kacheln und als Rückfall ohne `wgpu_render_state`), glow-Callback (Standardrenderer ist wgpu), eigene Plattform-Videofläche (zweiter Darstellungsweg je OS).

**Darstellungstakt (korrigiert, NR-Nr. 48):** Der Betrachter setzt `WgpuConfiguration { present_mode: AutoVsync, desired_maximum_frame_latency: Some(1) }` (kein Tearing, kein Absturzrisiko; Standard wäre bis zu 2 wartende Bilder). Schalter „Niedrigste Latenz (Bildrisse möglich)“ setzt beim nächsten Start des Betrachters `AutoNoVsync`. `Mailbox` wird nicht gesetzt (Absturz, wenn nicht unterstützt; eframe 0.29.1 fragt die Fläche vor dem Start nicht ab). Der Dekodier-Thread behält nur das neueste Bild und ruft `ctx.request_repaint()`. Messdefinition: Darstellungszeit = Übergabe an wgpu im Paint-Callback; die Präsentationswarteschlange danach ist nicht gemessen (Lücke ohne Wirkung auf die Entscheidung).

**Android:** `SurfaceView` mit MediaCodec-Decoder (`releaseOutputBuffer(index, true)` sofort, `KEY_LOW_LATENCY`); `tiles`-Bilder als `Bitmap.copyPixelsFromBuffer` in einer Compose-`Image`. Verworfen: `TextureView`.

**Binärpfad Kotlin↔Rust:** drei JNI-Funktionen mit direkten `ByteBuffer`s in `native/android-bridge/src/screen_jni.rs`: `screenPull(session: Long, dst: ByteBuffer, timeoutMs: Int): Int` (16-Byte-Kopf `pts_us` u64, `flags` u32, `len` u32, danach Nutzlast; `dst` ist der MediaCodec-Eingangspuffer; Rückgabe −1 Zeitüberschreitung, −2 Ende, −3 zu klein mit nötiger Größe in `len`), `screenPush(session, src, offset, length, ptsUs, flags): Int`, `screenControl(session, timeoutMs): String?`. Annahme A13 (NR-Nr. 50): `jni` 0.22.4 stellt `get_direct_buffer_address`/`get_direct_buffer_capacity` bereit; sonst Aufruf der JNI-Standardfunktionen `GetDirectBufferAddress`/`GetDirectBufferCapacity` (JNI 1.4) über `jni::sys`. Abnahmesignale `screen_task_android_viewer_pull_buffer_too_small`, `screen_task_android_viewer_decodes_barcode`. S2 legt in `native/android-bridge/src/io.rs` einen eigenen Binärpfad mit `ByteArray` an; beide Module bestehen nebeneinander.

Quellen C4: NR N3, N4, N20, N22; Bestand F5, F6.

### C5 — Eingabe und „was läuft“

**Eingabe**

| Plattform | Gewählt | Verworfen | Ehrliche Anzeige |
|---|---|---|---|
| Windows | `SendInput` im Hostprozess (Medium IL): Maus absolut mit `MOUSEEVENTF_ABSOLUTE` und `MOUSEEVENTF_VIRTUALDESK`, Tasten `KEYEVENTF_SCANCODE`, Text `KEYEVENTF_UNICODE` | UIAccess (Signatur, `\Program Files\`, nicht für Nicht-Assistenzsoftware); SYSTEM-Dienst (TODO) | Integritätsprüfung des Vordergrundprozesses vor jedem Batch (`OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `OpenProcessToken` → `TokenIntegrityLevel`); höher als eigene **oder Abfrage scheitert** → Eingabe verworfen, `blocked: "elevated_window"` (korrigiert, NR-Nr. 52: Tokenzugriff auf erhöhte Prozesse aus Medium IL ist unbelegt; Annahme A14, Ausfall führt zur sicheren Seite); sicherer Desktop → `blocked: "secure_desktop"`; Strg+Alt+Entf wird nicht angeboten |
| Linux X11 | XTest über `x11rb`; Zeichen außerhalb der Belegung über vorübergehende Belegung eines freien Keycodes (Verfahren nach xdotool, eigene Umsetzung; korrigiert, NR-Nr. 69: xdotool ist BSD-3-Clause, eine Übernahme wäre mit Hinweis erlaubt, gewählt ist eine eigene Umsetzung des Verfahrens) | uinput | Zeichen ohne Belegungsmöglichkeit einzeln gemeldet |
| Linux Wayland | RemoteDesktop-Portal mit ScreenCast in derselben Portal-Sitzung (`notify_pointer_motion_absolute`, `NotifyPointerButton`, `NotifyPointerAxis`, `notify_keyboard_keycode`, `NotifyKeyboardKeysym`, Touch); GNOME (Wiederherstellung ab 45) und KDE | libei (`ConnectToEIS`); uinput | wlroots-Desktops (Sway, Hyprland, river, Wayfire, phosh) haben kein RemoteDesktop-Portal → `control_unavailable` (NR N11); Steuern in einer nur zum Ansehen gestarteten Sitzung verlangt eine neue Portal-Freigabe |
| Android | Bedienungshilfe: `dispatchGesture` (API 24), `continueStroke` (26), `performGlobalAction` (Liste C3), `ACTION_SET_TEXT`, `ACTION_IME_ENTER` (30) | Shizuku (TODO) | keine Tastenereignisse; „Eingeschränkte Einstellungen“ bei Sideload (Android 13+; Android 15 verschärft, NR N16) |

Gemeinsam: Steuern exklusiv je Host (`control_busy`); gedrückte Tasten/Knöpfe werden bei Ende, Abgabe und Verbindungsverlust gelöst; Zwischenablage und Ton nicht Teil dieses Strangs (TODO).

**Betrachter-Tastatur (neu, NR N4):** Werkzeugleisten-Knöpfe für Tasten ohne egui-Ereignis: Super/Windows (links), Menü, Druck, Pause, Rollen, Feststell, NumLock, Wiedergabe/Pause, Lauter, Leiser, Stumm; angezeigt nur, wenn der Host die Taste anbietet. Nummernblock-Tasten kommen als Haupttasten an (egui-Grenze, angezeigt). Während der Betrachter Tastatur aufnimmt, setzt er `PlatformOutput::ime`; übertragen wird `Text` aus `ImeEvent::Commit`.

**„Was läuft“**

| Plattform | Quelle | Fensterauswahl |
|---|---|---|
| Windows | `EnumWindows` (sichtbar, nicht `DWMWA_CLOAKED`, mit Titel), Prozess (`QueryFullProcessImageNameW`), PID, Monitor, `IsIconic`, Fokus | ja (ab 1903) |
| Linux X11 | EWMH `_NET_CLIENT_LIST`, `_NET_WM_NAME`, `_NET_WM_PID`, `WM_CLASS`, `_NET_ACTIVE_WINDOW`; ohne EWMH-Fenstermanager `QueryTree` der Wurzel mit `WM_NAME` (korrigiert, NR-Nr. 59: EWMH-Spezifikation nicht abgerufen → Annahme A15; Abnahmesignal `screen_task_x11_window_list_ewmh_and_querytree` mit und ohne Fenstermanager) | ja |
| Linux Wayland | (1) `ext-foreign-toplevel-list-v1` (Sway, Hyprland, niri, COSMIC, Labwc, Jay, Louvre, Mir, phoc, river, Treeland; NR N12); (2) `wlr-foreign-toplevel-management`; (3) KDE: `org_kde_plasma_window_management` über `wayland-protocols-plasma` 0.3.12, freigegeben durch `X-KDE-Wayland-Interfaces=org_kde_plasma_window_management` in `smart-explorer.desktop` (korrigiert, NR-Nr. 61; Sekundärbeleg → Annahme A6: KWin ordnet über den absoluten `Exec`-Pfad zu, deshalb startet der Daemon den Hostprozess aus dem GUI-Binary); (4) GNOME: keine Fensterliste für Drittprogramme (Ref D2) → laufende Apps aus systemd-Einheiten `app[-<launcher>]-<ApplicationID>-<RANDOM>.scope` und `app[-<launcher>]-<ApplicationID>[@<RANDOM>].service` (korrigiert, NR-Nr. 62), App-ID durch Abgleich mit vorhandenen `.desktop`-Dateien (mit und ohne Launcher-Segment); Annahme A7: GNOME und KDE halten die Konvention ein | nur über den Portaldialog am Host |
| Android | Bedienungshilfe `getWindows()` (Flag `flagRetrieveInteractiveWindows` in der Dienstkonfiguration), sonst `UsageStatsManager` | App-Wahl im Systemdialog (ab 14 QPR2) |

Die Liste gehört zum Recht „Bildschirm ansehen“.

Quellen C5: Ref D1–D3; NR N4, N11–N13, N16, N18; NR-Nr. 51–63, 69.

### C6 — Gruppen

Gewählt L1: Kachelansicht je Raum bzw. „Alle Geräte“; Vorschau als `tiles`-Bild (lange Kante 320 Pixel, größere Kacheln fordern größer an), quittungsgetaktet (höchstens eine Vorschau je Gerät unterwegs), Abstand wählbar (Standard 1 s; auf Android-Hosts mindestens 333 ms durch das System, NR N16); Klick hebt dieselbe Verbindung per `SET_MODE` von `preview` auf `full`. Verworfen: kleine H.264-Ströme je Kachel (Dauerlast je Gerät), reine Liste als Hauptweg. Abschätzung (Annahme A1): typisch < 10 KB je Aktualisierung, schlimmstenfalls 12 Geräte × 75 KB/s; Host < 1 % CPU. Android-Hosts liefern Vorschauen nur mit aktiver se-Bedienungshilfe; ohne sie zeigt die Kachel die Vordergrund-App und „Ansehen anfragen“. Bei `Ask` zeigt die Kachel „Ansehen anfragen“ statt Bild. Rückwechsel: bisherige Vollansicht fällt auf `preview` zurück und bleibt verbunden, solange die Gruppenansicht offen ist.

Quellen C6: Bestand F3; NR N16; C3.

### C7 — Rechte, Rückfrage, Anzeige, Abbruch, Protokoll (über S1)

Gewählt: die S1-Fähigkeiten `ScreenView` und `ScreenControl` (`Off`/`Ask`/`Allow`, Standard `Off`) je Direktfreigabe und je Raummitglied; „Gültig bis“ gilt auch hier (abgelaufen = `Off`); Kontoprofil und „Nur lesen“ gelten nicht. `ScreenControl` setzt eine Sicht voraus: eine Steuersitzung braucht `ScreenView ≠ Off` und `ScreenControl ≠ Off`.

Ablauf am Host: Handshake → `prove_relation` (Transkript-Präfix `smart-explorer/share-screen/v1|`) → `capability_for(…, ScreenView|ScreenControl, now)` → bei `Ask` Rahmen `APPROVAL_PENDING` an den Betrachter und Rückfrage über den S1-Dienst `access_approval` („Diesmal erlauben“ = diese Sitzung; „Immer erlauben“ setzt die Fähigkeit auf `Allow`; „Ablehnen“ → `approval_denied`); keine Frist; Abbruch durch den Betrachter → `approval_cancelled`. Desktop: S1-Fenster `smart_explorer --access-consent <ID>` mit Text „{Gerät} möchte den Bildschirm ansehen/steuern“ plus Desktop-Benachrichtigung. Android: S1-Benachrichtigung; für Bildschirmanfragen müssen die erlaubenden Aktionen `PendingIntent.getActivity` auf die S4-Aktivität `ScreenConsentActivity` sein und beim Erstellen den Hintergrundstart ausdrücklich gewähren (Ziel API 36; NR N15) — Anforderung an S1, E1.

Fehlercodes im `HELLO_ERROR` (an S1 angeglichen): `permission_denied` (vor dem Beweis oder ohne Recht; keine Auskunft an Fremde), `right_not_granted` (Steuern verlangt, nur Ansehen erlaubt), `approval_denied`, `approval_cancelled`, `no_display`, `indicator_unavailable`, `control_busy`, `control_unavailable`, `host_shutdown`; beim Betrachter zusätzlich `screen_unsupported` (C3).

Anzeige am Host, solange eine Sitzung läuft: se-Anzeigefenster `smart_explorer --screen-indicator` (immer im Vordergrund, „Beenden“) auf Windows (zusätzlich gelber WGC-Rahmen) und Linux (Wayland: normales Fenster plus Portal-Anzeige); Android: Statusleisten-Chip plus Vordergrund-Benachrichtigung „Beenden“. Neu (NR N22): Findet der Daemon kein GUI-Binary (`TerminalOnly`, Daemon läuft als `se`), lehnt der Host jede Sitzung mit Bildschirminhalt mit `indicator_unavailable` ab („Bildschirmfreigabe braucht die Desktop-App auf dem Gerät“); die Testquelle `synthetic` (kein Bildschirminhalt) bleibt erlaubt. Grund: K5 verlangt, dass der Host sieht, dass er angesehen wird.

Abbruch: Betrachter schließt; Host beendet im Anzeigefenster, im Desktop-Tab **Bildschirm**, mit `se screen stop`, in der Android-Benachrichtigung; `CapabilityNarrowed(ScreenView|ScreenControl)` und `GrantExpired` beenden betroffene Sitzungen über `invalidate_restrictions_at` (`0x5345`); Daemon-Stopp → `host_shutdown`.

Protokoll: `record_access(AccessEntry { kind: AccessKind::Screen { mode, event }, principal, outcome, approval, … })` mit Ereignissen `session_started`, `control_taken`, `control_released`, `session_ended` (Grund, Dauer, Bytes); gleiche Dateien und Rotation wie Exec (`exec-audit.log`), Anzeige in den drei S1-Protokollansichten mit Art „Bildschirm“. Kein eigenes `screen-audit.log` (bisheriger Rückfall entfällt).

Verworfen: Bildschirmrecht an Exec koppeln; nur Rückfrage ohne gespeichertes Recht; eigenes Zustimmungsfenster `--screen-consent` (ersetzt durch den gemeinsamen S1-Dienst).

Quellen C7: S1-E, `strang-rechte.md` C5 und C7; NR N15, N22; Bestand F3.

### C8 — Bausteine und Lizenzen

| Baustein | Lizenz | Entscheidung | Grund |
|---|---|---|---|
| Pipeline (Protokoll, Regelung, Sitzungen, Oberflächen) | eigen (MIT) | Neubau | sitzt auf Iroh, S1-Rechten, se-Oberflächen |
| `windows` 0.58 | MIT/Apache | Features | vorhanden |
| `x11rb` 0.13.2, `wayland-client` 0.31, `wayland-protocols` 0.32, `wayland-protocols-wlr` 0.3, `wayland-protocols-plasma` 0.3 | MIT/Apache bzw. MIT | direkt in Lock-Version | Aufnahme, Eingabe, Fensterlisten |
| `ashpd` 0.13.13 | MIT | neu (Linux) | Portal-Anfrage/Antwort-Muster |
| PipeWire | MIT (Systembibliothek) | eigene Lade-ABI + SPA-POD in Rust | `pipewire`-Crate bindet zur Bauzeit |
| `openh264` 0.9.9, `openh264-sys2` 0.9.9 | BSD-2-Clause | neu (Windows, Linux), Feature `libloading` | lädt Cisco-Datei, kein C-Code |
| `wide` | Lizenz aus crates.io zu übernehmen (Annahme A16: Zlib OR Apache-2.0 OR MIT; geprüft in SCR-M10) | neu, Pflichtabhängigkeit von `openh264` | — |
| `bzip2` 0.6.1, `libbz2-rs-sys` | MIT OR Apache-2.0; `bzip2-1.0.6` (bei Binärverteilung kein Hinweis verlangt) | neu (Windows, Linux) | Entpacken der Cisco-Datei |
| `lz4_flex` 0.14, `libloading` 0.8, `png` 0.18, `egui-wgpu` | MIT, ISC, MIT/Apache, MIT/Apache | direkt in Lock-Version | Kachel-Codec, Laden, Schnappschuss, Shader |
| OpenH264-Binärdatei | Cisco-Binärlizenz v1.0 | Laufzeitdownload durch den Nutzer; alle vier Bedingungen erfüllt (SCR-M3, SCR-M10) | Patentdeckung nur so |
| scrcpy | Apache-2.0 | nicht in diesem Stapel | — |
| IronRDP / FreeRDP | MIT/Apache / Apache-2.0 | nicht verwendet | IronRDP-Server ohne H.264 (NR-Nr. 67); FreeRDP ohne Mehrwert |
| Eingebaute RDP-Server | proprietär / GPL-2.0+ | nicht verwendet | Windows Home ohne Host (Ref E2), keine Android-Entsprechung, Einrichtung je Desktop uneinheitlich; Konsolenübernahme auf Client-Windows ist Annahme ohne Wirkung (NR-Nr. 68) |
| RustDesk, Weylus, Deskreen | AGPL-3.0 | ausgeschlossen | K7 |
| Sunshine, Moonlight, gnome-remote-desktop | GPL | ausgeschlossen, nur öffentlich beschriebene Konzepte | K7 |
| xdotool | BSD-3-Clause (korrigiert, NR-Nr. 69) | Verfahren eigen umgesetzt, kein Code übernommen | kein Hinweis nötig |

Quellen C8: Ref E1, E2; NR N8, N9, N25; NR-Nr. 66–69.

### C9 — Messbarkeit

**Messgrößen:** Ende-zu-Ende-Latenz (Übergabe an wgpu bzw. `releaseOutputBuffer` minus Aufnahmezeit, Uhrversatz aus `PING`/`PONG` mit kleinster Umlaufzeit; p50, p95, Maximum); Bildrate (Mittel, Minimum je Sekunde); Bitrate (Mittel, Maximum); Erholung nach Verlust; Korruption (Bilder mit falscher Barcode-Prüfsumme, Soll 0); Textschärfe `static_psnr_db`.

**Werkzeug:** Testquelle `synthetic` (bewegtes Rechteck, rollender Text, statischer Textblock, Barcode mit `frame_id` und CRC-32); `se screen bench (GERÄT | --local) [--codec auto|h264|tiles] [--size BxH] [--fps N] [--seconds N] [--json]`.

**Ohne reale Geräte:** Share-Paar auf einem Runner; Verlust/Verzögerung mit `tc netem` auf `lo` — Annahme A17 (NR-Nr. 70: `sch_netem` im Runner-Kernel unbelegt); netem auf `lo` wirkt in beiden Richtungen, also 12,5 ms je Richtung für 25 ms Zusatz-Umlaufzeit (NR N21). Ist netem nicht ladbar, meldet die Suite `screen_task_netem_recovery` als „nicht ausgeführt“ (kein Bestehen); unabhängig davon prüft `screen_task_recovery_after_injected_reset` die Erholung über einen nur im Test vorhandenen Reset-Injektor zwischen zwei Endpunkten im selben Prozess. Relay mit `SE_SHARE_RELAY_ONLY=1`. Echte Aufnahme: X11 über Xvfb; Wayland über kopfloses Sway + `xdg-desktop-portal-wlr` (`chooser_type = none`, `output_name`) mit per `apt` installiertem PipeWire; KDE-Steuern nur, wenn KWin kopflos startet (`kde-authorized`/`remote-desktop` im Permission-Store); Windows über die interaktive Runner-Sitzung (WGC, SendInput; MF fehlt); Android-Emulator mit `appops … PROJECT_MEDIA allow`.

**Schwellen (Annahme A1, Frage je Schwelle: „ist die Kette auf CI-Hardware echtzeitfähig?“):** 1280 × 720 bei 30 Bildern/s mit Software-Kodierung: Bildrate ≥ 27; p95 ≤ 150 ms auf `lo`; bei 2 % Verlust und 25 ms Zusatz-Umlaufzeit Erholung ≤ 1 s, Korruption 0; Bitrate-Mittel ≤ 1,2 × Ziel; `static_psnr_db` ≥ 38 dB für `h264`, unendlich für `tiles`.

Quellen C9: Ref C2; NR N7, N11, N21; `native/review-task-device.py:61-179`.

## D — Meilensteine (Umsetzungsreihenfolge)

Gemeinsam: neue Rust-Dateien < 500 Zeilen (Schätzung in Klammern); `N/screen/core/` ohne `cfg`, OS-Crates und FFI; Plattformcode in `N/screen/os/{windows,linux_os,android,native_ffi,shared}`, Auswahl in `N/screen/os/mod.rs`; `native_ffi` ist für Windows und Linux gemeinsam (`#[cfg(any(windows, target_os = "linux"))]`), nicht für Android; jede Oberflächenänderung aktualisiert `docs/oberflaechen/bildschirm.md` im selben Commit; nach nativen Änderungen Graphify-Aktualisierung nach AGENTS.md. Abnahmesignale werden nur in der einen Suite (SCR-M12) ausgeführt. Programmnamen: `smart_explorer` (Windows installiert `Smart Explorer.exe`).

### SCR-M1 — Kernformate (plattformneutral)

- **Ergebnis:** kein Bedienweg; geprüfte Kernbausteine.
- **Neue Dateien:** `N/screen/mod.rs` (60), `core/mod.rs` (40), `core/wire.rs` (300), `core/wire_hello.rs` (250: Hello-Typen, Abbildung auf `RelationServerHello`/`RelationClientHello`, Präfix `smart-explorer/share-screen/v1|`), `core/wire_control.rs` (350), `core/wire_binary.rs` (250), `core/input_model.rs` (300: HID-Tabelle inkl. Werkzeugleisten-Tasten), `core/negotiate.rs` (250), `core/tiles_codec.rs` (400), `core/pixel_convert.rs` (350), `core/synthetic.rs` (300), `core/rate_control.rs` (350), `core/pacing.rs` (250: Wächter inkl. Relay-Grenze 768 KiB), `core/metrics.rs` (300), `core/session_state.rs` (250), `core/window_list.rs` (150), Tests `core/*_tests.rs`.
- **Geänderte Dateien:** `N/lib.rs` (`mod screen;`), `N/share/core/exec_protocol.rs` (private `read_payload`/`write_payload` → `pub(crate)`), `native/Cargo.toml` (`lz4_flex = "0.14"`).
- **Verträge:** ALPN, Rahmen, Eingabe-Batch (C3); Exec-Format unverändert.
- **Oberflächen:** keine. **Abhängigkeiten:** S1 PRM-M1 nur für die Hello-Abbildung (Typen `RelationServerHello`/`RelationClientHello`).
- **Plattformprüfung:** Big-Endian-Rahmen auf Little-Endian-Zielen (Windows x64, Linux x64, Android arm64/x86_64) explizit.
- **Abnahmesignal:** `screen_task_wire_roundtrip_all_tags`, `screen_task_frame_limits_reject_oversize`, `screen_task_negotiate_matrix`, `screen_task_tiles_lossless_roundtrip`, `screen_task_barcode_detects_corruption`, `screen_task_rate_control_backs_off_on_delay_gradient`, `screen_task_rate_control_loss_rules` (> 10 % senken, 2–10 % halten, < 2 % × 1,05), `screen_task_pacing_drops_before_encode`, `screen_task_pacing_relay_guard_below_relay_budget`, `screen_task_pacing_resets_stale_gop`, `screen_task_input_release_on_end`.
- **Release:** `lz4_flex` (im Lock).

### SCR-M2 — Bildschirmverbindung mit S1-Rechten, Rückfrage, Anzeige und Testbild

- **Ergebnis:** Im Zugriffsdialog (S1) stellt der Host-Nutzer für ein Gerät „Bildschirm ansehen“/„Bildschirm steuern“ auf Aus/Fragen/Erlauben (Zeilen erscheinen nur auf Hosts mit grafischer Sitzung). Das andere Gerät misst mit `se screen bench HOST` die Strecke mit dem Testbild; bei „Fragen“ erscheint das S1-Rückfragefenster. Laufende Sitzungen zeigt Desktop-Tab **Geräte & Freigaben → Bildschirm** (mit „Beenden“, „Alle beenden“ und offenen Bildschirm-Rückfragen) und `se screen sessions`; beendet wird dort oder mit `se screen stop`; das Anzeigefenster zeigt „Bildschirm wird angesehen von …“.
- **Neue Dateien:** `N/screen/core/host_backend.rs` (200: Traits `ScreenHostBackend`, `ScreenSource`, `ScreenEncoder`, `ScreenDecoder`; `ConsentBroker` wird von S1 `access_approval` umgesetzt), `core/host_server.rs` (450), `core/host_pipeline.rs` (400), `core/gop_stream.rs` (300: inkl. sofortigem `stop` zurückgesetzter Streams), `core/viewer_client.rs` (450), `core/registry.rs` (300: Sitzungen, exklusives Steuern, `restrict_authorization` für `CapabilityNarrowed` und `GrantExpired`), `core/authorize.rs` (200: `prove_relation`, `capability_for(…, now)`, `record_access`), `core/catalog_entries.rs` (120: `register_screen_entries(entries, facts)` mit Bedingung `facts.graphical_session`), `core/bench.rs` (300), `N/screen/os/shared/synthetic_backend.rs` (200), `os/shared/indicator.rs` (200: startet `smart_explorer --screen-indicator` über den Geschwisterpfad; fehlt das GUI-Binary → `indicator_unavailable`), `N/share/core/node_screen.rs` (150), `N/daemon/os/shared/ipc_protocol_screen.rs` (250), `N/daemon/os/shared/screen_ipc.rs` (400), `N/daemon/os/shared/screen_ipc_client.rs` (300), `N/cli/screen.rs` (250), `N/cli/screen/bench.rs` (200), `N/cli/screen/sessions.rs` (200), `N/cli/screen/output.rs` (250), `N/cli/completions_screen.rs` (150), `N/app/core/screen_indicator_ui.rs` (200), `N/app/core/share_screen_tab_ui.rs` (300), `docs/oberflaechen/bildschirm.md`.
- **Geänderte Dateien:** `N/share/core/node.rs` (ALPN, Backend-Feld; 459 → ≈ 470), `N/share/core/node_accept.rs` (Verteilung, Strafe wie Exec, Fehlerart), `N/share/core/node_restrictions.rs` (`screen_registry().restrict_authorization`), `N/share/core/access_catalog.rs` (Aufruf `register_screen_entries`, Schnittstelle von S1 vorgesehen), `N/daemon/os/shared/ipc_protocol.rs` (eine Anfrage- und eine Antwortvariante; S1 PRM-M3 fügt vier Varianten hinzu — liegt die Datei danach über 500 Zeilen, wandern die Bildschirmvarianten vollständig nach `ipc_protocol_screen.rs` und `ipc_protocol.rs` erhält nur den Verweis), `N/cli/mod.rs`, `N/cli/completions.rs`, `N/lib.rs` (Modus `--screen-indicator`), `N/app/core/share_pages_ui.rs` (Tab 9 **Bildschirm**, Tabs 0–8 unverändert), `N/cli/mcp/catalog.rs` (Beispiel `se screen bench`), `docs/oberflaechen/INDEX.md`.
- **Verträge neu:** Handshake `ScreenServerHello{protocol_version, challenge, server_device_id, server_public_key, server_fingerprint, server_node_id}`, `ScreenClientHello{protocol_version, relation_kind, relation_id, device_id, device_name, public_key, fingerprint, node_id, client_nonce, proof, mode, source, decoders, display_size, max_fps}`, `ScreenHelloOk{authorization, platform, encoders, input, monitors, limits, approval: "not_required"|"once"|"always"}`, `ScreenHelloError{code}` (Codes C7); IPC `ScreenIpcRequest::{Probe, Stream, Bench, HostSessions, HostStop, HostWatch}` (Rückfrageentscheidungen über S1 `DecideAccessApproval`); CLI `se screen devices [--room RAUM] [--probe] [--json]`, `se screen probe GERÄT [--json]` → `{"device","supported","platform","view","control","monitors":[…],"input":{…},"codecs":[…],"limits":[…]}`, `se screen bench … [--json]` → `{"codec","encoder","decoder","path","frames_sent","frames_shown","fps_mean","fps_min","latency_ms":{"p50","p95","max"},"bitrate_kbps":{"mean","max"},"keyframes","recoveries":[…],"corrupt_frames","static_psnr_db"}`, `se screen sessions [--json]` → `{"sessions":[{"id","device_id","device_name","relation","room_id","mode","source","codec","encoder","path","started_at","bitrate_kbps","fps","controlling","state"}]}` (`state` umfasst `waiting_for_approval`), `se screen stop (SITZUNG | --all) [--json]`; Rechte über S1: `se share grants access SELEKTOR --screen-view off|ask|allow --screen-control off|ask|allow`, JSON `screen_view`/`screen_control`. **Erhalten:** FS-, Exec- und LAN-Link-ALPN, Endpoint-Konfiguration, Exec-Strafen, Tabnummern 0–8.
- **Oberflächen:** CLI (Text, JSON, Vervollständigung), Desktop (Tab 9, Anzeigefenster; Rechtezeilen und Rückfragefenster aus S1), IPC, MCP über `se` (Rückfrageentscheidung über MCP gesperrt, S1 C7), Netz, Doku. Android: noch keine Bildschirmzeilen (Host erst ab SCR-M8).
- **Abhängigkeiten:** S1 PRM-M1 (Typen, Beweis, Katalog), PRM-M3 (Rückfragedienst, Fenster, IPC), PRM-M4 (Protokoll), PRM-M8/PRM-M9 (CLI-Optionen und Zugriffsdialog zeigen Katalogzeilen); SCR-M1.
- **Plattformprüfung:** Uni-Streams je Verbindung (NR N1 belegt); Relay mit Reset und Wächter; Anzeigefenster unter Windows, X11, Wayland; Linux-Daemon als `se` mit und ohne Geschwister-GUI; root-Worker → `no_display`.
- **Abnahmesignal:** `screen_task_pair_bench_tiles`, `screen_task_relay_only_bench`, `screen_task_netem_recovery` (nur mit `sch_netem`), `screen_task_recovery_after_injected_reset`, `screen_task_reset_streams_free_slots`, `screen_task_permission_denied_without_right`, `screen_task_view_only_control_right_not_granted`, `screen_task_ask_once_session`, `screen_task_ask_always_sets_allow`, `screen_task_ask_deny_approval_denied`, `screen_task_viewer_cancel_withdraws_approval`, `screen_task_narrowing_closes_session`, `screen_task_grant_expired_closes_session`, `screen_task_audit_screen_entries`, `screen_task_indicator_unavailable_refuses` (Installation `TerminalOnly`; `synthetic` erlaubt), `screen_task_catalog_rows_only_with_graphical_session`, `screen_task_old_peer_unsupported` (Code `0x178`), `screen_task_fs_exec_unchanged`, `screen_task_cli_json_shapes`, `screen_task_completions_screen`.
- **Release:** keine neue Feed-Datei; GUI-Binary erhält den Modus `--screen-indicator`.

### SCR-M3 — H.264-Kette Desktop und Hostprozess

- **Ergebnis:** Desktop-Hosts und -Betrachter nutzen H.264: Windows mit Media Foundation, wo vorhanden; Linux und Windows ohne MF nach Nachladen von OpenH264 in **Bildschirm → Kodierung** oder mit `se screen codec openh264 enable`. Vor dem Download zeigt se den vollständigen Cisco-Lizenztext, an der Steuerstelle steht „OpenH264 Video Codec provided by Cisco Systems, Inc.“, der Schalter erlaubt Aus- und Wiedereinschalten (Bedingungen 1–3; Bedingung 4 in SCR-M10). Aufnahme und Kodierung laufen in einem Hostprozess; ein Absturz dort beendet nur die Bildschirmsitzung.
- **Neue Dateien:** `N/screen/core/codec_registry.rs` (250), `N/screen/os/shared/codec_inventory.rs` (200: MF-Erkennung zur Laufzeit, OpenH264-Profile nach A4), `os/shared/host_worker_client.rs` (350: startet das GUI-Binary mit `--screen-host-worker`, Rückfall `current_exe()` nur ohne GUI-Binary für `synthetic`), `os/shared/host_worker_main.rs` (300), `N/screen/os/windows/worker_job.rs` (120), `N/screen/os/linux_os/worker_death.rs` (80), `N/screen/os/windows/mf_common.rs` (300), `mf_encoder.rs` (450), `mf_encoder_sync.rs` (250), `mf_decoder.rs` (400), `N/screen/os/native_ffi/openh264_codec.rs` (400: über `openh264::OpenH264API::from_blob_path`), `native_ffi/openh264_install.rs` (350), `native_ffi/openh264_pins.rs` (60: Version 2.6.0, URLs, vier SHA-256-Werte aus C2), `N/cli/screen/codec.rs` (200), `N/app/core/share_screen_codec_ui.rs` (250).
- **Geänderte Dateien:** `native/Cargo.toml` (`windows`-Features `Win32_Media_MediaFoundation`, `Win32_Graphics_Direct3D`, `Win32_Graphics_Direct3D11`, `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`, `Win32_System_Com_StructuredStorage`, `Win32_System_Variant` (Namen belegt, NR N5); unter `[target.'cfg(any(windows, target_os = "linux"))'.dependencies]`: `openh264 = { version = "0.9.9", default-features = false, features = ["libloading"] }`, `bzip2 = "0.6"`, `libloading = "0.8"`), `N/bin/se.rs` und `N/lib.rs` (interner Aufruf `--screen-host-worker` als einziges Argument), `N/screen/core/host_pipeline.rs`.
- **Verträge neu:** Hostprozess-Rahmen (Tag/Länge); CLI `se screen codec [--json]` → `{"encoders":[{"codec","profiles","name","hardware"}],"decoders":[…],"openh264":{"state":"absent|enabled|disabled|failed","version":"2.6.0","notice":"OpenH264 Video Codec provided by Cisco Systems, Inc."}}`, `se screen codec openh264 enable [--yes] | disable | remove`; IPC `ScreenIpcRequest::{Codecs, OpenH264}`.
- **Oberflächen:** CLI, Desktop, IPC, MCP über `se` (Aktivieren über MCP nur auf ausdrücklichen Nutzerwunsch, Katalogregel `N/cli/mcp/catalog.rs:36-38`), Doku. Android nicht betroffen.
- **Abhängigkeiten:** SCR-M2.
- **Plattformprüfung:** Windows mit Hardware-MFT, mit Software-MFT, ohne MF (Runner); Linux x64 mit OpenH264; Ende des Daemons beendet den Hostprozess (Job Object bzw. `PR_SET_PDEATHSIG`).
- **Abnahmesignal:** `screen_task_local_bench_h264_openh264` (Linux und Windows), `screen_task_pair_bench_h264`, `screen_task_openh264_pin_mismatch_refuses`, `screen_task_openh264_disable_unloads`, `screen_task_openh264_decodes_constrained_high` (Testdatei High ohne B-Bilder, auf dem Linux-Runner mit dort per `apt` installiertem `ffmpeg`/x264 erzeugt; x264 ist nur Prüfwerkzeug), `screen_task_codec_inventory_without_mf` (Windows-Runner), `screen_task_mf_encode_decode` (nur auf einem Rechner mit Media Foundation; auf GitHub-Runnern „nicht ausgeführt“, K6), `screen_task_worker_crash_keeps_daemon`, `screen_task_negotiate_cb_high_for_openh264_viewer`.
- **Release:** neue Crates `openh264`, `openh264-sys2`, `wide`, `bzip2`, `libbz2-rs-sys` (Lizenzpflichten SCR-M10); kein mitgelieferter Codec.

### SCR-M4 — Windows als Host

- **Ergebnis:** Ein Windows-PC zeigt einen Monitor (wählbar) oder ein Fenster; `se screen windows GERÄT` listet Fenster, `se screen snapshot GERÄT --out bild.png` speichert ein verlustfreies Standbild; MCP-Werkzeug `screen` liefert Geräte, Fenster, Standbild. Bei UAC/Sperre zeigt der Betrachter „Windows zeigt den sicheren Desktop – Anzeige und Eingabe sind durch Windows gesperrt“; bei minimiertem Fenster „Fenster ist minimiert“.
- **Neue Dateien:** `N/screen/os/windows/d3d_device.rs` (200), `capture_wgc.rs` (450: nur `CreateForMonitor`/`CreateForWindow`), `capture_dxgi.rs` (400), `gpu_convert.rs` (350), `monitors.rs` (200), `window_list.rs` (300), `secure_desktop.rs` (150), `host_backend.rs` (350), `N/screen/core/snapshot.rs` (200), `N/cli/screen/windows.rs` (200), `N/cli/screen/snapshot.rs` (200), `N/cli/mcp/screen_tool.rs` (400).
- **Geänderte Dateien:** `native/Cargo.toml` (`windows`-Features `Graphics`, `Graphics_Capture`, `Graphics_DirectX`, `Graphics_DirectX_Direct3D11`, `Win32_System_WinRT`, `Win32_System_WinRT_Graphics_Capture`, `Win32_System_WinRT_Direct3D11`, `Win32_Graphics_Dwm`, `Win32_UI_HiDpi`, `Win32_System_StationsAndDesktops`; `png = "0.18"`), `N/cli/mcp/catalog.rs` (Werkzeug `screen`; 351 → ≈ 370), `N/cli/screen.rs`.
- **Verträge neu:** Quellen `monitor:<id>`, `window:<id>`, `synthetic`; `se screen windows GERÄT [--json]` → `{"windows":[{"id","title","app","pid","monitor","minimized","focused"}],"apps":[{"app_id","name","pid"}],"list_source":"enum_windows|ewmh|x11_query_tree|ext_foreign_toplevel|wlr_foreign_toplevel|kde_plasma_window_management|systemd_units|accessibility_windows|usage_stats","limits":[…]}`; `se screen snapshot GERÄT [--source QUELLE] --out DATEI.png [--json]` → `{"path","width","height","bytes","source"}`; MCP `screen` mit `action ∈ {devices, probe, windows, snapshot}` (Standbild auf das Antwortbudget `max_bytes` verkleinert).
- **Oberflächen:** CLI, MCP, IPC (`Windows`, `Snapshot`), Netz, Doku.
- **Abhängigkeiten:** SCR-M3.
- **Plattformprüfung:** Windows 10 1507–1809 (DXGI, keine Fensteraufnahme), 1903+ (WGC), 2004+ (Cursor-Schalter), Windows 11 Build 26100 (A8); gemischte DPI; minimiertes Fenster; getrennte Sitzung → `session_inactive`.
- **Abnahmesignal:** `screen_task_windows_wgc_snapshot_matches`, `screen_task_windows_window_list_has_test_window`, `screen_task_windows_minimized_state`, `screen_task_windows_bench_real_capture` (OpenH264 oder `tiles`), `screen_task_windows_interop_factories_only` (statisch: kein `TryCreateFromDisplayId`/`TryCreateFromWindowId` im Code), `screen_task_windows_lock_reports_secure_desktop` (letzter Schritt), `screen_task_mcp_screen_tool_schema`, `screen_task_mcp_snapshot_image_content`.
- **Release:** nur Binärgröße.

### SCR-M5 — Linux als Host: X11 und Wayland

- **Ergebnis:** Linux zeigt Bildschirm oder Fenster: X11 ohne Dialog (se-Anzeige), Wayland nach Systemdialog, danach ohne Dialog, solange der Token gilt (wlroots nur mit `XDPW_PERSIST_MODE`, Hinweis in der Oberfläche); „Freigabe vergessen“ im Tab **Bildschirm** und `se screen portal forget`; Fenster-/App-Liste je Desktop nach C5.
- **Neue Dateien:** `N/screen/os/linux_os/session_detect.rs` (120), `x11_capture.rs` (450), `x11_cursor.rs` (150), `x11_windows.rs` (300: EWMH und `QueryTree`-Rückfall), `x11_monitors.rs` (150), `portal_session.rs` (400), `portal_tokens.rs` (150), `pipewire_abi.rs` (350), `pipewire_stream.rs` (450), `spa_pod.rs` (300), `wayland_toplevels.rs` (350: ext und wlr), `kde_window_management.rs` (300), `app_units.rs` (250: systemd-Einheiten nach korrigierter Konvention), `host_backend.rs` (350), `N/cli/screen/portal.rs` (120).
- **Geänderte Dateien:** `native/Cargo.toml` unter `[target.'cfg(target_os = "linux")'.dependencies]`: `x11rb = { version = "0.13", features = ["shm", "damage", "xfixes", "randr", "composite", "xtest"] }`, `wayland-client = "0.31"`, `wayland-protocols = { version = "0.32", features = ["client", "staging"] }`, `wayland-protocols-wlr = { version = "0.3", features = ["client"] }`, `wayland-protocols-plasma = { version = "0.3", features = ["client"] }`, `ashpd = { version = "0.13", default-features = false, features = ["tokio", "screencast", "remote_desktop"] }`; `install-linux.sh` (Zeile `X-KDE-Wayland-Interfaces=org_kde_plasma_window_management` in `smart-explorer.desktop`; S2 ändert dieselbe Datei für `--with-fuse`), `N/app/core/share_screen_tab_ui.rs`.
- **Verträge neu:** `screen-portal-tokens.json` (`screencast`, `remote_desktop`, `saved_at`); `se screen portal forget [--json]`; Zustände `waiting_for_portal`, `no_display` mit Grund `pipewire_missing`.
- **Oberflächen:** CLI, Desktop, IPC, Doku.
- **Abhängigkeiten:** SCR-M3, SCR-M4.
- **Plattformprüfung:** X11 mit/ohne Compositor und mit/ohne EWMH-Fenstermanager; XWayland-`DISPLAY` in Wayland → Portal; GNOME, KDE, Sway; ohne PipeWire startet das Programm; Bibliotheksname `libpipewire-0.3.so.0` (NR N10).
- **Abnahmesignal:** `screen_task_x11_snapshot_matches`, `screen_task_x11_window_list_ewmh_and_querytree` (mit `openbox` und ohne Fenstermanager), `screen_task_x11_bench_real_capture`, `screen_task_spa_pod_enum_format_bytes` (Byte-Vergleich mit der Header-dokumentierten Struktur), `screen_task_wayland_portal_bench` (Sway kopflos + xdpw `chooser_type = none`), `screen_task_wayland_token_replaced_after_start` (`XDPW_PERSIST_MODE=permanent`), `screen_task_linux_without_pipewire_starts`, `screen_task_app_units_parse` (`app-gnome-org.gnome.Evince-12345.scope`, `app-KDE-org.kde.okular@12345.service`, `app-org.mozilla.firefox-1234.scope`, `app-firefox-1234.scope`), `screen_task_kde_window_list` (nur wenn KWin kopflos startet; sonst „nicht ausgeführt“).
- **Release:** neue Crates `ashpd` (Linux); `install-linux.sh` geändert; keine Bauabhängigkeit an Systembibliotheken.

### SCR-M6 — Eingabe auf Desktop-Hosts

- **Ergebnis:** Mit „Bildschirm steuern“ bedient das andere Gerät Maus, Tastatur, Text; der Betrachter sieht Sperren (Administratorfenster, sicherer Desktop, kein RemoteDesktop-Portal). `se screen input GERÄT --text …|--key …|--click X,Y|--move X,Y|--scroll DX,DY|--action NAME`; MCP `screen` mit `action: input`.
- **Neue Dateien:** `N/screen/os/windows/input_send.rs` (400), `input_integrity.rs` (200: Ausfall der Abfrage = gesperrt), `N/screen/os/linux_os/x11_input.rs` (400), `x11_keymap.rs` (350), `portal_input.rs` (300), `N/cli/screen/input.rs` (300).
- **Geänderte Dateien:** `N/screen/core/registry.rs`, `N/cli/mcp/screen_tool.rs`; `native/Cargo.toml` unverändert für `windows-sys` (`Win32_UI_Input_KeyboardAndMouse`, `Win32_Security` vorhanden, `native/Cargo.toml:155-196`).
- **Verträge neu:** `CONTROL_REQUEST{take|release}`, `CONTROL_STATE{holder, blocked}` mit `blocked ∈ {null, elevated_window, secure_desktop, control_unavailable, unsupported_on_host}`; Koordinaten: Zahl = Pixel der Quelle, Suffix `%` = Anteil; `--key` mit Tastennamen aus `input_model.rs`; JSON `{"delivered","blocked"}`.
- **Oberflächen:** CLI, MCP (verändernd/zerstörend annotiert), IPC (`Input`), Netz, Doku.
- **Abhängigkeiten:** SCR-M4, SCR-M5; S1 (`ScreenControl`).
- **Plattformprüfung:** Windows gleiche/erhöhte Stufe; X11 Zeichen außerhalb der Belegung; GNOME/KDE RemoteDesktop mit `persist_mode` 2; Sway → `control_unavailable`.
- **Abnahmesignal:** `screen_task_windows_sendinput_text_reaches_window` (UI Automation liest zurück), `screen_task_windows_integrity_query_failure_blocks` (Unit mit eingespeistem Fehler), `screen_task_windows_elevated_blocked_reported` (nur wenn der Runner ein erhöhtes Fenster neben Medium IL starten kann; sonst „nicht ausgeführt“, NR N7 Lücke), `screen_task_x11_xtest_text_and_click`, `screen_task_x11_unmapped_char_via_spare_keycode`, `screen_task_control_exclusive`, `screen_task_keys_released_on_disconnect`, `screen_task_wayland_control_unavailable_reported` (Sway), `screen_task_kde_remote_desktop_input` (KWin kopflos + Permission-Store; sonst „nicht ausgeführt“, K3).
- **Release:** keine.

### SCR-M7 — Desktop-Betrachter und Gruppenansicht

- **Ergebnis:** **Geräte & Freigaben**: Geräteseite **Bildschirm öffnen**/**Bildschirm steuern**, Raumseite **Bildschirme des Raums**, Tab **Bildschirm** → „Alle Geräte“. Im Betrachter: Gerät wechseln, Monitor/Fenster wählen, Steuern ein/aus, Einpassen/1:1/Vollbild, „Schärfe vor Flüssigkeit“, „Verlustfrei“, „Niedrigste Latenz (Bildrisse möglich)“, Statistik, Werkzeugleisten-Tasten (C5). `se screen view GERÄT [--source QUELLE] [--control] [--fullscreen]`, `se screen grid [--room RAUM]`.
- **Neue Dateien:** `N/app/core/screen_viewer_app.rs` (400: `WgpuConfiguration` nach C4), `screen_view_widget.rs` (450), `screen_wgpu_yuv.rs` (350), `screen_texture_cpu.rs` (150), `screen_toolbar_ui.rs` (350), `screen_input_map.rs` (400: inkl. IME), `screen_tile_grid.rs` (350), `screen_stats_overlay.rs` (150), `screen_decode_thread.rs` (300), `screen_launch.rs` (150: GUI-Binary über die Updater-Erkennung, NR N22), `N/cli/screen/view.rs` (150).
- **Geänderte Dateien:** `N/lib.rs` (`--screen-view`, `--screen-grid`; S2 ergänzt dort `--drive-restore`), `N/app/core/share_device_page_ui.rs` (390 → ≈ 405), `N/app/core/share_room_page_ui.rs` (338 → ≈ 350), `N/app/core/share_screen_tab_ui.rs`.
- **Verträge neu:** `smart_explorer --screen-view <ziel-json>` (`{"relation","relation_id","device_id","source","control"}`), `smart_explorer --screen-grid [--room <id>]`; `SET_MODE{mode, source, size, fps, sharpness, lossless}`; S5-Einbettung (E2). CLI `--json` → `{"launched":true,"pid"}`; ohne GUI-Binary Fehler „Desktop-App nicht installiert“.
- **Oberflächen:** Desktop, CLI, IPC (`Stream`), Doku; MCP: Live-Ansicht ausgenommen (Grund: MCP überträgt Werkzeugantworten, keine Videoströme).
- **Abhängigkeiten:** SCR-M3–M6.
- **Plattformprüfung:** wgpu DX12/Vulkan (Windows), Vulkan und GL (Linux); Wayland-Fenster; ohne `wgpu_render_state` CPU-Weg.
- **Abnahmesignal:** `screen_task_viewer_yuv_shader_matches_cpu` (±2), `screen_task_viewer_wgpu_config` (`AutoVsync`, `desired_maximum_frame_latency = Some(1)`, Schalter → `AutoNoVsync`), `screen_task_input_map_table` (alle `egui::Key` und alle Werkzeugleisten-Tasten → HID), `screen_task_ime_commit_sends_text`, `screen_task_grid_switch_reuses_connection`, `screen_task_viewer_launch_args`, GUI-Harness-Prüfung der neuen Knöpfe.
- **Release:** keine neue Datei.

### SCR-M8 — Android als Host

- **Ergebnis:** Ein Telefon lässt sich ansehen und bedienen: Rückfrage als S1-Benachrichtigung; „Freigeben“ öffnet direkt die se-Zustimmungsaktivität, danach Systemdialog „Bildschirm teilen“; laufende Sitzung mit Systemchip und se-Benachrichtigung „Beenden“. Tab **Teilen → Bildschirm dieses Telefons**: Sitzungen, Bedienungshilfe-Zustand (Knopf „Aktivieren“ mit Hinweis auf „Eingeschränkte Einstellungen“), Nutzungsdatenzugriff. Mit Bedienungshilfe: Gesten, Systemaktionen, Text, Vorschau und Standbild (≤ 3/s) ohne Systemdialog. Rechtezeilen erscheinen jetzt im Android-`AccessDialog.kt` (S1-Katalog).
- **Neue Kotlin-Dateien:** `A/screen/ScreenHostService.kt`, `A/screen/ScreenEncoder.kt`, `A/screen/ScreenConsentActivity.kt` (nimmt die S1-Entscheidung entgegen, ruft `createScreenCaptureIntent`, startet danach den Dienst), `A/screen/ScreenHostControl.kt`, `A/screen/ScreenControlService.kt`, `A/screen/GestureMapper.kt`, `A/screen/RunningApps.kt`, `A/screen/ScreenStillSource.kt` (Abstand ≥ 333 ms), `A/screen/SyntheticSurfaceSource.kt`, `A/screen/ScreenHostNotifier.kt` (laufende Sitzung), `A/ui/screen/ScreenHostSection.kt`, `A/api/ScreenApi.kt`; `android/app/src/main/res/xml/screen_control_service.xml` (`canPerformGestures`, `canRetrieveWindowContent`, `canTakeScreenshot`, `flagRetrieveInteractiveWindows`).
- **Neue Rust-Dateien:** `native/android-bridge/src/screen_jni.rs` (200), `N/screen/os/android/bridge_backend.rs` (350), `bridge_channels.rs` (250), `N/mobile/os/shared/domains/screen_host.rs` (300).
- **Geänderte Dateien:** `native/android-bridge/src/lib.rs` (`mod screen_jni;`; S2 ergänzt `mod io;`), `A/core/NativeBridge.kt`, `A/core/CoreEvents.kt` (`screenHostStart`, `screenSession`), `N/mobile/core/events.rs`, `N/mobile/os/shared/domains/mod.rs`, `android/app/src/main/AndroidManifest.xml` (`FOREGROUND_SERVICE_MEDIA_PROJECTION`, Dienst `mediaProjection`, Aktivität, Bedienungshilfe-Dienst mit `BIND_ACCESSIBILITY_SERVICE`), `strings.xml`, `A/ui/share/ShareScreen.kt`, `docs/superpowers/plans/2026-09-25-android-apk/api.md`; S1-Android-Benachrichtigung: Aktionsziel für Bildschirmfähigkeiten (E1 Punkt 7).
- **Verträge neu:** Bridge `screen.hostSessions`, `screen.hostStop {session|all}`, `screen.hostReady {session, width, height, encoder}`, `screen.hostEnded {session, reason}`, `screen.hostPrereqs` → `{"accessibility":"on|off|restricted","usageAccess":bool,"appWindowCapture":bool}`; Ereignisse `screenHostStart {session, source, maxWidth, maxHeight, fps, bitrate, profile, still}`, `screenSession {session, state}`; Rückfragen über S1 `share.accessApprovals`/`share.decideAccessApproval`; JNI `screenPush`, `screenControl`.
- **Oberflächen:** Android-GUI, Bridge, Netz, Doku; Desktop/CLI/MCP als Betrachter (Plattform `android`, `limits`: `consent_each_session`, `no_key_events`, `still_max_3_per_second`).
- **Abhängigkeiten:** SCR-M2; S1 PRM-M10.
- **Plattformprüfung:** API 30–33 und 34+; App-Fenster ab 14 QPR2; Ziel 36: `PendingIntent.getActivity` mit gewährtem Hintergrundstart; Bedienungshilfe bei Sideload 13/14/15; Sperre beendet Projektion.
- **Abnahmesignal:** `screen_task_android_host_bench_synthetic`, `screen_task_android_host_snapshot_nonblack` (`appops … PROJECT_MEDIA allow`), `screen_task_android_still_interval_respected`, `screen_task_android_still_secure_window_black` (Testaktivität mit `FLAG_SECURE`, A12), `screen_task_android_gesture_tap_lands`, `screen_task_android_text_set`, `screen_task_android_consent_action_is_activity_intent`, `screen_task_android_bridge_shapes`, Instrumentierung `ScreenTaskTest.kt`.
- **Release:** APK mit neuen Manifesteinträgen; keine native Bibliothek dazu.

### SCR-M9 — Android als Betrachter und Gruppenansicht

- **Ergebnis:** Tab **Teilen**: ⋮ **Bildschirm…** an einem Gerät (Vollbild, Zoom, Steuern, Fensterliste, Sonderaktionen, Tastatur), ⋮ **Bildschirme** an einem Raum (Kacheln, Tippen = Vollansicht).
- **Neue Dateien:** `A/ui/screen/ScreenViewScreen.kt`, `ScreenDecoder.kt`, `ScreenTilesScreen.kt`, `ScreenPreviewTile.kt`, `ScreenInputMapper.kt`, `N/mobile/os/shared/domains/screen_view.rs` (350), `N/screen/os/android/viewer_sink.rs` (250).
- **Geänderte Dateien:** `A/ui/AppNav.kt` (`NavRequest.OpenScreen(target)`, `NavRequest.ShowScreenGrid(roomId)`), `A/ui/share/SharePeers.kt`, `N/mobile/os/shared/domains/mod.rs`, `api.md`.
- **Verträge neu:** Bridge `screen.devices {roomId?}`, `screen.probe`, `screen.windows`, `screen.viewOpen {target, mode, source}` → `{session}`, `screen.viewSetMode`, `screen.viewInput {session, events}`, `screen.viewClose`; JNI `screenPull`.
- **Oberflächen:** Android-GUI, Bridge, Doku.
- **Abhängigkeiten:** SCR-M2, SCR-M7, SCR-M8.
- **Plattformprüfung:** Decoder-Niedriglatenz ab API 30; Rückfall ohne `KEY_LOW_LATENCY`; Touch → Zeiger (Desktop-Hosts) bzw. Gesten (Android-Hosts).
- **Abnahmesignal:** `screen_task_android_viewer_decodes_barcode`, `screen_task_android_viewer_pull_buffer_too_small`, `screen_task_android_grid_switch`.
- **Release:** wie SCR-M8.

### SCR-M10 — Lizenzpflichten (Release)

- **Ergebnis:** Jede Lizenzpflicht der neuen S4-Bausteine ist an allen Verteilwegen erfüllt; in **Bildschirm → Kodierung → Lizenzen** und mit `se screen codec licenses` sind die Texte lesbar.
- **Pflichten (NR N8, N25):** Cisco-Binärlizenz v1.0 Bedingung 4: vollständiger Text samt dieser Bedingung an der Lizenzstelle; `openh264`/`openh264-sys2` (BSD-2-Clause): Hinweis bei Binärverteilung in Dokumentation bzw. Begleitmaterial; `ashpd` (MIT): Copyright- und Lizenzhinweis; `wide` und `bzip2` (MIT/Apache bzw. A16): Hinweis; `libbz2-rs-sys`: keiner verlangt, trotzdem genannt.
- **Neue Dateien:** `third-party/openh264/BINARY_LICENSE.txt` (Cisco v1.0 wörtlich), `third-party/openh264-rs/LICENSE` (BSD-2 aus dem Crate), `third-party/ashpd/LICENSE`, `third-party/wide/LICENSE`, `third-party/bzip2-rs/LICENSE`, `N/screen/core/licenses.rs` (150: Texte per `include_str!` eingebettet, damit Installer, Linux-Skript, Update-Feed und Programm ohne neue Feed-Dateien dieselben Texte tragen).
- **Geänderte Dateien:** `native/installer.nsi` (`licenses\OpenH264\BINARY_LICENSE.txt`, `licenses\openh264-rs\LICENSE.txt`, `licenses\wide\LICENSE.txt`, `licenses\bzip2-rs\LICENSE.txt`, dazu `Delete` im Deinstallationsteil wie bei Dokany `:325-329`), `N/cli/screen/codec.rs` (`licenses`), `N/app/core/share_screen_codec_ui.rs` (Ansicht), `docs/RELEASING.md` (Abschnitt Lizenztexte neben Dokany `:100-113`), `README.md` (Abschnitt OpenH264 mit vollständigem Cisco-Text als Verweis auf `third-party/openh264/BINARY_LICENSE.txt` und Hinweis „persönliche, nicht vergütete Nutzung“ der AVC-Lizenz).
- **Nicht in S4:** die bestehende allgemeine Lücke, dass das Release keine Lizenztexte der übrigen Rust-Crates verteilt (NR N25, auch S2 N25) → TODO `RUST-CRATE-LICENSES` mit Vorschlag eines generierten Verzeichnisses; Entscheidung beim Hauptagenten, weil sie alle Stränge und alle Crates betrifft.
- **Abhängigkeiten:** SCR-M3, SCR-M5.
- **Plattformprüfung:** Windows-Installer, Linux-Skript und Update-Feed (Texte im Binary), Android (keine der neuen Crates im APK; `cfg` geprüft).
- **Abnahmesignal:** `screen_task_license_notices_complete` (Remote-`cargo metadata`: Lizenzfeld jedes neu hinzugekommenen Crates hat einen eingebetteten Text; prüft A16), `screen_task_cisco_license_verbatim` (SHA-256 des eingebetteten Textes = Pin des abgerufenen v1.0-Textes), `screen_task_installer_lists_screen_licenses` (statisch, `installer.nsi`), `screen_task_android_has_no_openh264` (statisch, `cfg` der Abhängigkeit).
- **Release:** Installer legt vier zusätzliche Dateien ab; Feed unverändert.

### SCR-M11 — Doku und Inventur

- **Ergebnis:** `docs/oberflaechen/bildschirm.md` nennt jede Fähigkeit je Oberfläche mit Beleg und Ausnahmen (Live-Video nicht über MCP; Rückfrageentscheidung nicht über MCP; Wayland-Fensterwahl am Host; wlroots ohne Steuern; Android ohne Tastenereignisse; Windows ohne UAC/Strg+Alt+Entf).
- **Geänderte Dateien:** `docs/oberflaechen/bildschirm.md`, `docs/oberflaechen/INDEX.md`, `docs/ARCHITEKTUR.md`, `README.md`, `docs/GOTCHAS.md` (PipeWire-Laden und SPA-POD, OpenH264-Pins, WGC-Rahmen und Interop-Fabriken, Uni-Stream-Zählplätze, Relay-Grenze), `docs/TODO.md` (SYSTEM-Hilfsdienst mit SAS-Richtlinie, UIAccess, Shizuku-`UserService`, AV1/HEVC, Zwischenablage, Ton, libei, NVENC Linux, `SCREEN-VAAPI` mit den offenen Punkten aus NR N14, `RUST-CRATE-LICENSES`), `api.md`, `docs/SHARE_SERVER.md` (Relay verwirft bei voller Ziel-Warteschlange; Video-Wächter), `N/cli/mcp/catalog.rs`.
- **Abnahmesignal:** Stale-Suche nach AGENTS.md; `screen_task_docs_inventory_lists_every_cli_command`.
- **Release:** keine.

### SCR-M12 — Remote-Task-Suite

- **Einstieg:** `.github/workflows/screen-task.yml` → `native/test-screen-task.py` (Präfix `screen_task_`), Hilfsskripte `native/screen-task-linux.py`, `native/screen-task-windows.py`, `native/screen-task-android.py`, Instrumentierung `android/app/src/androidTest/java/app/smartexplorer/android/task/ScreenTaskTest.kt`.
- **Jobs (Zeitlimit je Job ≥ 30 min):** `unit` (Linux, Windows); `linux-x11` (Xvfb, `openbox`, Paar, `tiles`/`h264`, netem falls ladbar, Relay, Snapshot, Fensterliste, XTest, `ffmpeg`-Testdatei für A4); `linux-wayland` (per `apt`: `sway`, `pipewire`, `wireplumber`, `xdg-desktop-portal`, `xdg-desktop-portal-wlr`; Versuch KWin kopflos für KDE-Prüfungen); `windows` (`windows-2025`: Paar, WGC/DXGI, OpenH264, ohne MF, SendInput, Sperre als letzter Schritt); `android` und `android-device` (Emulator mit `appops`, Bedienungshilfe per `settings put secure enabled_accessibility_services` — Annahme A18, NR N16 Lücke; Gerätepaar nach `review-task-device.py`); in allen Paar-Jobs `screen_task_fs_exec_unchanged`. Jede bedingte Prüfung meldet „nicht ausgeführt“ mit Grund, nie „bestanden“.
- **OpenH264 in CI:** Download aus der festgelegten Quelle mit Pin-Prüfung im Job.
- **Offen für den Hauptagenten (K6):** zusätzliche Stufe auf einem Windows-Client-Rechner mit Media Foundation und GPU für `screen_task_mf_encode_decode`.

**Gesamtauswirkung Release/Installer/Feed/CI:** keine neue Feed-Datei; Installer +4 Lizenzdateien; `install-linux.sh` eine Zeile in der `.desktop`-Datei; neue Crates `ashpd` (Linux), `openh264`, `openh264-sys2`, `wide`, `bzip2`, `libbz2-rs-sys` (Windows, Linux); direkte Nutzung vorhandener Lock-Crates; MSRV von `ashpd` 1.87 (CI nutzt `stable`; `rmcp` verlangt bereits 1.88); `cargo audit` deckt die neuen Crates; neuer Workflow `screen-task.yml`. VA-API entfällt in diesem Stapel.

## E — Schnittstellen als Verträge

### E1 — S4 ↔ S1 (übernommen aus S1-E, mit Abweichungen zur früheren S4-Erwartung)

1. **Fähigkeiten:** `Capability::{ScreenView, ScreenControl}` in `N/share/core/access_grant.rs`, `CapabilityDecision { Off, Ask, Allow }`, `CapabilityState { decision, policy_revision, changed_at }`; je Direktfreigabe und je Raummitglied. Abweichungen: kein getrenntes Registrierverfahren für Fähigkeiten (sie stehen fest im Enum); keine weiteren Geltungsbereiche; `policy_revision`/`changed_at` gelten für das ganze Zugriffsrecht; „Gültig bis“ gilt auch für Bildschirmfähigkeiten (neu für S4: Ablauf beendet Sitzungen).
2. **Beweis und Prüfung:** `prove_relation(&RelationServerHello, &RelationClientHello, remote_node, &ShareAuthState) -> Result<ProvenPrincipal, RelationRejection>` mit Transkript-Präfix als Feld (S4: `smart-explorer/share-screen/v1|`; S4 bildet seine Hello-Typen darauf ab); `capability_for(&ShareAuthState, &ProvenPrincipal, Capability, now: i64) -> CapabilityState`. Abweichungen: zusätzlicher Parameter `now`; protokollneutrale Hello-Typen statt Exec-Hellos.
3. **Katalog und Oberflächen:** `capability_catalog(&HostFacts) -> Vec<CapabilityEntry>`; S4 liefert `register_screen_entries(&mut Vec<CapabilityEntry>, &HostFacts)` (`N/screen/core/catalog_entries.rs`), Bedingung `HostFacts.graphical_session`; CLI `se share grants access … --screen-view off|ask|allow --screen-control off|ask|allow`, JSON `screen_view`/`screen_control`, Bridge `screenView`/`screenControl` über `share.setAccess`, Desktop-Zugriffsdialog und `AccessDialog.kt`. Abweichung: Die CLI-Befehlsform ist `se share grants access` (nicht eigener `se share grants screen`).
4. **Entzug:** `RestrictionReason::CapabilityNarrowed(Capability)` und `RestrictionReason::GrantExpired` über `invalidate_restrictions_at`; S4 hängt `screen_registry().restrict_authorization(epoch, restrictions)` ein. Abweichung: zusätzlich `GrantExpired`.
5. **Protokoll:** `record_access(AccessEntry { kind: AccessKind::Screen { mode, event }, principal, outcome, approval, … })` in `N/share/os/shared/access_audit.rs`, Dateien `exec-audit.log`/`exec-audit-rejected.log`. Abweichung: kein Rückfall auf ein eigenes `screen-audit.log`.
6. **Rückfragedienst:** S1 `access_approval` setzt den S4-Trait `ConsentBroker` um (`request(principal, capability, summary) -> ApprovalTicket`, Entscheidung `once|always|deny`, Abbruch durch den Betrachter); Fenster `smart_explorer --access-consent <ID>`; IPC `AccessApprovals`/`DecideAccessApproval`; MCP darf nicht entscheiden. Abweichungen: S4 verzichtet auf eigenes Zustimmungsfenster; „Immer erlauben“ ist neu; eine offene Bildschirm-Rückfrage belegt eine Reservierung der Exec-Registry (bestehende Grenzen 2 je Gerät, 8 gesamt; `N/share/core/exec_registry.rs:96-97`) — S4 übernimmt das (Flutschutz ohne neue Grenze), Wirkung: eine offene Bildschirm-Rückfrage zählt gegen die Exec-Rückfragen desselben Geräts.
7. **Neu von S4 an S1 (Android, NR N15):** Für `ScreenView`/`ScreenControl` müssen die erlaubenden Aktionen der S1-Benachrichtigung `PendingIntent.getActivity` auf `A/screen/ScreenConsentActivity.kt` sein und beim Erstellen den Hintergrundstart gewähren (`ActivityOptions.setPendingIntentCreatorBackgroundActivityStartMode(MODE_BACKGROUND_ACTIVITY_START_ALLOWED)`), weil MediaProjection eine Aktivität im Vordergrund verlangt; die Aktivität meldet die Entscheidung über `share.decideAccessApproval`. Abnahmesignal `screen_task_android_consent_action_is_activity_intent`.
8. **Bedingung an S1 (erfüllt laut S1-E):** Der Kontodienst ersetzt den Daemon in der Nutzersitzung nicht als Bildschirm-Host.

### E2 — S4 → S5

Desktop: (1) Startargumente `smart_explorer --screen-view <ziel-json>` und `smart_explorer --screen-grid [--room <id>]` (Windows `Smart Explorer.exe`), gleichwertig `se screen view`/`grid`; (2) im selben Prozess `pub(crate)` aus `N/app/core/`: `ScreenViewWidget::new(ScreenTarget, ViewOptions)`, `show(&mut self, &mut egui::Ui) -> ViewResponse` (`Closed`, `SwitchDevice(ScreenTarget)`, `StateChanged(ViewState)`), `ScreenTileGrid::new(GridScope::{AllDevices, Room(String)})`, `show(&mut self, &mut egui::Ui) -> Option<ScreenTarget>`; Voraussetzung: S5 nutzt eframe/wgpu im selben Crate und setzt für eine eingebettete Ansicht `desired_maximum_frame_latency = Some(1)` in seiner eigenen `NativeOptions`; (3) `crate::screen::device_states(scope) -> Vec<DeviceScreenState { target, name, platform, online, supported, view, control, limits }>` über IPC `Probe`.

Android: (4) `ScreenView(target, control, modifier)`, `ScreenPreviewTile(target, intervalMs, onOpen, modifier)` (auf Android-Hosts mindestens 333 ms), `ScreenTilesGrid(scope, onOpen)` in `A/ui/screen/`; `NavRequest.OpenScreen`, `NavRequest.ShowScreenGrid`; (5) `ScreenApi.devices(roomId)`, `probe(target)`, `windows(target)`. Keine Home-Screen-AppWidgets von S4 (Annahme: AppWidgets tragen keine MediaCodec-Fläche); Standbild-Widgets entscheidet S5.

### E3 — S2 und S3

Keine funktionale Abhängigkeit. Berührungspunkte bei der Zusammenführung: `N/lib.rs` (S2 `--drive-restore`, S4 `--screen-*`), `native/android-bridge/src/lib.rs` (S2 `mod io;` mit `ByteArray`, S4 `mod screen_jni;` mit direkten `ByteBuffer`s), `install-linux.sh` (S2 `--with-fuse`, S4 `X-KDE-Wayland-Interfaces`), `docs/RELEASING.md` und TODO `RUST-CRATE-LICENSES` (S2 N25). Läuft der Daemon in der S2-Rolle „nur Laufwerke“, gilt für die Erreichbarkeit des Bildschirm-Hosts dasselbe wie für den Share-Knoten in dieser Rolle (S4 stellt keine eigene Anforderung).

## F — Risiken und Annahmen

**Annahmen mit Wirkung und Prüfung:**

| Nr. | Annahme | Wirkung, wenn falsch | Prüfung |
|---|---|---|---|
| A1 | Schwellen und Startwerte (C3, C6, C9) | Suite misst zu streng oder zu lasch | erste Remote-Messung; Werte dann festschreiben |
| A2 | Wiederholbilder schärfen Text nach | Text bleibt unscharf | `static_psnr_db`; Rückfall „Verlustfrei“ |
| A3 | Patentdeckung der OS-Codecs beim OS-Hersteller (NR-Nr. 20) | rechtliches Risiko (R12) | nicht prüfbar; keine Patentaussage in der Doku |
| A4 | OpenH264 dekodiert Constrained High (NR-Nr. 28) | Linux-Betrachter erhalten nur `cb` | `screen_task_openh264_decodes_constrained_high`; Regel in C2 |
| A5 | Hardware-MFTs nehmen Bitrate/Schlüsselbild zur Laufzeit an | Regelung wirkt nicht | `screen_task_mf_encode_decode` nur auf Rechner mit MF (K6) |
| A6 | KWin gibt `org_kde_plasma_window_management` über `X-KDE-Wayland-Interfaces` frei und ordnet über den `Exec`-Pfad zu (NR-Nr. 61) | KDE-Fensterliste fehlt; Rückfall systemd-Einheiten | `screen_task_kde_window_list` (bedingt) |
| A7 | GNOME/KDE benennen App-Einheiten nach systemd-Konvention (NR-Nr. 62) | App-Liste leer → Anzeige „keine Programmliste verfügbar“ | `screen_task_app_units_parse` (Format), reale Desktops ungeprüft |
| A8 | Interop-Fabriken ohne Zustimmung auch auf 24H2/25H2 | Windows-Aufnahme scheitert | `screen_task_windows_wgc_snapshot_matches` auf Build 26100 |
| A9 | WGC liefert auf dem sicheren Desktop keine verwertbaren Bilder (NR-Nr. 8) | Erkennung über Desktopname greift trotzdem | `screen_task_windows_lock_reports_secure_desktop` |
| A10 | Portal-Persistenz ohne Host-Registrierung | wiederholte Dialoge | `screen_task_wayland_token_replaced_after_start` |
| A11 | `pw_buffer`/`spa_buffer`-Layout stabil über 0.3.x–1.6 | Absturz im Hostprozess (isoliert) | `screen_task_wayland_portal_bench` auf der Runner-Version |
| A12 | `FLAG_SECURE`-Inhalte schwarz in `takeScreenshot` (NR-Nr. 19) | sichere Inhalte sichtbar | `screen_task_android_still_secure_window_black` |
| A13 | `jni` 0.22.4 bietet direkte Puffer (NR-Nr. 50) | Rückfall über `jni::sys` | Android-E2E-Signale |
| A14 | Tokenzugriff auf erhöhte Prozesse aus Medium IL (NR-Nr. 52) | Abfrage scheitert → Eingabe gesperrt (sichere Seite), auch bei nicht erhöhten Fenstern mit geschütztem Prozess | `screen_task_windows_integrity_query_failure_blocks`; `…_elevated_blocked_reported` bedingt |
| A15 | EWMH-Atome wie beschrieben (NR-Nr. 59) | X11-Liste leer → `QueryTree`-Rückfall | `screen_task_x11_window_list_ewmh_and_querytree` |
| A16 | Lizenz von `wide` | Hinweistext falsch | `screen_task_license_notices_complete` |
| A17 | `sch_netem` im Runner-Kernel (NR-Nr. 70) | Verlustprüfung über netem entfällt | „nicht ausgeführt“ + `screen_task_recovery_after_injected_reset` |
| A18 | Bedienungshilfe per `settings put secure` im Emulator aktivierbar | Android-Steuerprüfung entfällt | `screen_task_android_gesture_tap_lands` meldet „nicht ausgeführt“ |
| A19 | Minimierte Fenster liefern keine Bilder (NR-Nr. 6) | keine Wirkung (Erkennung über `IsIconic`) | `screen_task_windows_minimized_state` |

Annahmen ohne Wirkung auf eine Entscheidung (Gründe verworfener Lösungen): KMS braucht `CAP_SYS_ADMIN` (NR-Nr. 14), HEVC braucht unter Windows eine Store-Erweiterung (NR-Nr. 23), RDP übernimmt die Konsolensitzung (NR-Nr. 68).

**Risiken:**

| Nr. | Risiko | Gegenmaßnahme |
|---|---|---|
| R1 | S1-Meilensteine PRM-M1/M3/M4 verspäten sich | SCR-M1 und Kodiererteile von SCR-M3 vorziehen; ALPN erst mit Rechten registrieren |
| R3 | eigene PipeWire-ABI/SPA-POD weicht ab | Prozessisolation, Versionsprüfung, `no_display` mit Grund |
| R4 | Cisco ändert Dateien unter gleicher Version | Pin-Fehler → kein Laden, Meldung mit Version; Pins nur mit Release ändern |
| R5 | GPU-/MF-Pfade in CI ungeprüft | K6-Entscheidung des Hauptagenten; Bericht „ungeprüft“ |
| R7 | Hersteller ignorieren `KEY_LATENCY`/`KEY_LOW_LATENCY` | Messung zeigt Codec-Namen; Software-Codec wählbar |
| R10 | Weiterleitung über Daemon-IPC kostet Latenz | gleiche Strecke in `se screen bench` gemessen |
| R11 | begrenzte Hardware-Kodiersitzungen | Software-Rückfall, sonst Fehler mit Grund |
| R12 | Patentlage OS-Codecs (A3) | keine Patentaussagen; Nutzerhinweis zur Cisco-Lizenz |
| R13 | „Eingeschränkte Einstellungen“ (Android 13+, verschärft in 15) | geführter Bedienweg |
| R14 | Bildschirminhalte sensibel | `Off` als Standard, Anzeige Pflicht (`indicator_unavailable`), Protokoll, Entzug, synthetische Messquelle |
| R17 | Zusammenführung mit S1 (`exec_protocol.rs`, `ipc_protocol.rs`) und S2 (`lib.rs`, Bridge, `install-linux.sh`) | Dateigrenzen in D genannt; `ipc_protocol.rs` > 500 → Auslagerung |

(R2 und R16 entfallen nach NR N1 bzw. N8; R6 ist in R5 aufgegangen; R8, R9, R15 sind als A2, Werkzeugleisten-Tasten bzw. C4-Korrektur erledigt.)

## G — Restlücken nach der Nachrecherche

Keine weitere Nachrecherche für diesen Stapel; die folgenden Lücken sind über Annahmen (F) oder TODO behandelt: Latenzspitzen beim Iroh-Pfadwechsel (NR N19, nur messbar → Bench mit `SE_SHARE_RELAY_ONLY` und Statistik); Präsentationslatenz je wgpu-Modus (NR N3, nur messbar); Hersteller-MFT-Unterstützung von Constrained Baseline (NR N6 → A5/K6); Plasma-Version der KDE-Wiederherstellung und KWin kopflos auf `ubuntu-24.04` (NR N11 → K3, A6); VA-API-Teilmenge (NR N14 → TODO `SCREEN-VAAPI`); Android-16-Stand der SystemUI-Projektionsprüfung (NR N15, Emulator-Abbild der Suite zeigt es); Lizenz der Shizuku-API und `screenrecord`-Latenz (NR N17 → TODO); Microsoft-Beleg für SYSTEM-Helfer auf dem Winlogon-Desktop (NR N18 → TODO).

## Prüfprotokoll Nachrecherche

**Widerlegte Aussagen (NR-Nr.):**

| NR-Nr. | Bisherige Aussage | Behandlung in dieser Fassung |
|---|---|---|
| 5 | `windows-capture` bringe eine zweite große Bindungsschicht | C1 Windows L4: Kostengrund gestrichen (`windows` 0.62.2 schon im Lock); Verwerfung bleibt mit „kein Funktionsgewinn“ und Typbruch 0.62↔0.58 |
| 19 | `takeScreenshot` liefert bei `FLAG_SECURE` Fehler | C1 Android L2 korrigiert; Annahme A12 mit `screen_task_android_still_secure_window_black`; 333-ms-Grenze in C1, C6, SCR-M8, E2 |
| 28 | OpenH264 dekodiert nur Constrained Baseline | C2 Dekodierer/Aushandlung: OpenH264-Betrachter melden `cb` und `high`; Annahme A4 mit Abnahmesignal und festgelegter Rückfallregel |
| 37 | Relay-Verlust erscheint nur als Verzögerung; FEC wirkt dort nicht | C3: FEC-Begründung korrigiert; neuer Relay-Wächter 768 KiB (`screen_task_pacing_relay_guard_below_relay_budget`); B ergänzt |
| 39 | `read_payload`/`write_payload` seien wiederverwendbar (`pub(crate)`) | B und C3: heute privat; SCR-M1 ändert die Sichtbarkeit; Zusammenführung mit S1 PRM-M2 genannt |
| 48 | `AutoVsync` addiert höchstens eine Bildperiode | C4: `desired_maximum_frame_latency = Some(1)`, Schalter `AutoNoVsync`, kein Mailbox; `screen_task_viewer_wgpu_config`; R15 entfällt |
| 61 | KWin biete keine Fensterliste für Drittprogramme | C5: KDE-Weg über `X-KDE-Wayland-Interfaces` und `wayland-protocols-plasma`; Annahme A6; `install-linux.sh` in SCR-M5; `screen_task_kde_window_list` |
| 62 | systemd-Konvention `app-<id>-<n>.scope`/`app-<id>@<n>.service` | C5 korrigiert auf `app[-<launcher>]-<ApplicationID>…`; `app_units.rs`; `screen_task_app_units_parse`; Annahme A7 |
| 69 | xdotool sei GPL | C5/C8: BSD-3-Clause; eigene Umsetzung des Verfahrens, kein Hinweis nötig |
| 74 | Programmname `smart-explorer --screen-…` | überall `smart_explorer` (Windows `Smart Explorer.exe`); Geschwister-Erkennung; neuer Zustand `indicator_unavailable` (C7, SCR-M2) |

**Nicht belegbare Aussagen (NR-Nr.):**

| NR-Nr. | Aussage | Behandlung |
|---|---|---|
| 6 | Minimierte Fenster liefern keine Bilder | Entscheidung unabhängig gemacht (`IsIconic`); A19; `screen_task_windows_minimized_state` |
| 8 | Sicherer Desktop aus Nutzerprozess nicht lesbar (WGC-Teil) | A9; Erkennung über `OpenInputDesktop`/Desktopname; `screen_task_windows_lock_reports_secure_desktop` |
| 14 | KMS braucht `CAP_SYS_ADMIN` | Annahme ohne Wirkung; Verwerfung trägt K5 |
| 20 | Patentdeckung der OS-Codecs | A3, R12; K7 als Lizenzkriterium davon getrennt |
| 23 | HEVC unter Windows braucht Store-Erweiterung | Annahme ohne Wirkung; Verwerfung trägt „kein Bedarf über H.264 hinaus“ |
| 25 | Windows-N ohne Media Foundation | MF-Erkennung zur Laufzeit statt nach Edition; `screen_task_codec_inventory_without_mf` |
| 50 | `jni` 0.22.4 mit direkten Puffern | A13 mit Rückfall `jni::sys`; Android-E2E-Signale |
| 52 | Integritätsprüfung über `TokenIntegrityLevel` | A14; Ausfall = gesperrt; `screen_task_windows_integrity_query_failure_blocks` |
| 59 | EWMH-Atome | A15; `QueryTree`-Rückfall; `screen_task_x11_window_list_ewmh_and_querytree` |
| 68 | RDP übernimmt die Konsolensitzung | Annahme ohne Wirkung; Verwerfung trägt Windows Home/Android/Einrichtung |
| 70 | `tc netem` auf `lo` im Runner | A17; „nicht ausgeführt“ statt Bestehen; `screen_task_recovery_after_injected_reset` |

**Weitere aus NR übernommene Präzisierungen:** NR-Nr. 3 (nur Interop-Fabriken, `screen_task_windows_interop_factories_only`), NR-Nr. 10/56 (wlroots ohne RemoteDesktop, Persistenz nur mit `XDPW_PERSIST_MODE`), NR-Nr. 12 (SPA-POD in Rust, `screen_task_spa_pod_enum_format_bytes`), NR-Nr. 21/75 (Cisco-Bedingung 4, Crate mit `libloading`, `openh264_abi.rs` entfällt, SCR-M10), NR-Nr. 33 (4:4:4 bei Hardware-MFTs ungeprüft, ohne Entscheidungswirkung), NR-Nr. 42/43 (verlustbasierte Steigerung 5 %, Statistik als Differenz), NR-Nr. 53 (SAS-Richtlinie im TODO), NR-Nr. 63 (`flagRetrieveInteractiveWindows` in der Dienstkonfiguration), NR-Nr. 65 (Android-Aktion als Aktivitäts-`PendingIntent`, E1 Punkt 7), NR-Nr. 71/72 (CI-Wege Wayland/Windows), NR N1 (Uni-Stream-Zählplätze, `screen_task_reset_streams_free_slots`), NR N2 (`0x178`), NR N4 (Werkzeugleisten-Tasten, IME), NR N8 (Pins), NR N9 (Feature-Liste `ashpd`), NR N14 (VA-API aus dem Stapel genommen), NR N22 (Daemon als `se`, Geschwister-GUI), NR N23 (DXGI für 1507–1809), NR N25 (Lizenzpflichten SCR-M10, allgemeine Crate-Lücke als TODO).

**NR-Folgen 1–14 (Abschnitt 3 der Nachrecherche):** 1 → C2 Aushandlung/Dekodierer; 2 → C3 FEC und Wächter; 3 → C1 L4; 4 → C1 Android L2; 5 → C4 Darstellungstakt; 6 → C5 KDE und Scopes; 7 → C1 Linux L1/C8 (SPA-POD); 8 → C1 Persistenz, C5 Wayland-Steuern; 9 → C3 Rahmenformat; 10 → C8, SCR-M10; 11 → K6, SCR-M3, SCR-M12; 12 → C4/C7 Programmnamen, `indicator_unavailable`; 13 → C1/C5 TODO SAS-Richtlinie; 14 → C1 Fabrikwahl.
