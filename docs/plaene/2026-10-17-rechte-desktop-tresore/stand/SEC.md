# Stand SEC (Sicherheitsagent)

Stand: 2026-10-19.

- Committet sind Punkt 1–3, 1b und 4 (df15feb5, e6b4c24a, 25e08110, e2233d85, c87bd083).
- Punkt 4b und Punkt 5 (U22) sind im Quelltext fertig, aber nicht gebaut, nicht getestet und nicht committet.
- Jede geänderte Rust-Datei ist mit `rustfmt --check` geprüft und hat weniger als 500 Zeilen.

## Punkt 4b

- `screen/os/mod.rs`: Android-Module eingetragen; `platform_backend()` hat je einen Arm für Windows, Linux, Android und alle übrigen Plattformen; `host_backend()` nutzt auf Android `BridgeBackend` ohne `WorkerBackend`; `host_indicator()` ist neu.
- R9-Zeilen gesetzt in `mobile/os/shared/domains/mod.rs`, `mobile/mod.rs` und `android-bridge/src/lib.rs`.
- `PermTaskTest.kt` prüft die drei Aktionen der Bildschirm-Rückfrage.
- In `screen/core/host_backend.rs` ist der Kommentar zu den Reservierungen gestrichen.

## Punkt 5: U22

**Entscheidung zum Ort der Eingabe (Frage aus U16-4):** Die Eingabe läuft im Hostprozess über `worker_wire` `INPUT` → `INPUT_DONE`. Der Daemon-`WorkerBackend` leitet sie an den jüngsten Hostprozess einer Sitzung im Modus `control` weiter. Ob eine Sitzung steuern darf, liest `WorkerBackend` beim Öffnen aus dem Register. Das Register bekommt er über `host_indicator`.

**Umgesetzt:**
- Windows: `input_send.rs`, `input_integrity.rs` und Tests.
- Linux: `x11_input.rs`, `x11_keymap.rs` und `portal_input.rs` (RemoteDesktop-Sitzung mit ScreenCast).
- Gemeinsam: `os/shared/input_keys.rs`.
- CLI: `cli/screen/input.rs`.
- MCP: eigenes Werkzeug `screen_input` in `cli/mcp/screen_tool.rs`.
- Lizenz: `third-party/openh264/BINARY_LICENSE.txt` und `screen/core/licenses.rs`.
- `se screen codec licenses` und das Fenster „Lizenzen“ in der Desktop-App.

**Registrierungsbedarf:** siehe Abschlussbericht vom 2026-10-19 (Lauf 4b/5). Betroffen sind `screen/core/mod.rs`, `cli/screen.rs`, `daemon/mod.rs`, `cli/mcp/catalog.rs`, `cli/mcp/server.rs`, `.gitattributes` und `installer.nsi` (U36).

## Nächster Schritt

Auf Punkt 6 warten.
