# REG-B: Registrierungen von U22 (2026-10-19)

Quelle: `SEC.md`, Punkt 5 (U22), Registrierungsbedarf. Nicht gesetzt: Punkt 2 (`cli/screen.rs`, U25) und Punkt 8 (`installer.nsi`, U36). Keine Builds, Tests, Commits oder Pushes; Formatprüfung je Rust-Datei mit `rustfmt --check` ohne Ausgabe.

| Datei | Eintrag |
| --- | --- |
| `native/src/screen/core/mod.rs` | `pub(crate) mod licenses;` (nach `input_model`) |
| `native/src/daemon/mod.rs` | `pub(crate) use ipc_protocol::screen::StreamRequest;`; `screen_stream` und `StreamEvent` in den vorhandenen `pub(crate) use screen_ipc_client::{…}` aufgenommen |
| `native/src/cli/mcp/catalog.rs` | Werkzeug `screen_input` (`changes(true, false)`, Pflichtfeld `device`); Testliste der Namen um `screen_input` ergänzt; Ausnahmeliste der nicht vorab geladenen Werkzeuge um `screen_input` ergänzt |
| `native/src/cli/mcp/server.rs` | Arm `"screen_input"` neben `"screen"` |
| `.gitattributes` | `/third-party/openh264/BINARY_LICENSE.txt -text` |

Abweichung: im Test `mcp_task_catalog_is_complete_and_within_host_limits` steht `screen_input` zusätzlich in der Liste der Werkzeuge ohne `alwaysLoad` (`file_tool`-Ausnahme); sonst erwartet der Test `alwaysLoad`, das `UPFRONT` für `screen_input` nicht setzt.
Offen: Der Anleitungstext `INSTRUCTIONS` nennt die nachzuladenden Werkzeuge ohne `screen_input`.
