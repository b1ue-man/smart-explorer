# REG-C: Registrierungen von U26 (R9, 2026-10-20)

Quelle: `U26.md`, Abschnitt 4 (Registrierungsbedarf). Keine Builds, Tests, Commits oder Pushes; Formatprüfung je Rust-Datei mit `rustfmt --check --edition 2021` ohne Ausgabe. Pfade relativ zu `native/src/`.

| Datei | Eintrag |
| --- | --- |
| `cli/mod.rs` | `mod vault;` (nach `mod update;`); Variante `Vault(vault::VaultArgs)` mit `about = "Cryptomator vaults: list, unlock, lock, create, sync, compare"` nach `Screen`; Arm `Command::Vault(args) => vault::run(args),` nach `Command::Screen` |
| `cli/mcp/catalog.rs` | `INSTRUCTIONS`: Ziel `` `vault://<site>/path` (unlocked vault) `` nach `` `gdrive:///path` ``; Gruppe `` vaults (`vault`) `` zwischen `drive` und `se Desktop`; Werkzeug `se`: Beispiel `["vault","list","--json"]` nach `["desktop","enable","--json"]` |
| `cli/mcp/status_tools.rs` | in `status()` der Schlüssel `"vaults": crate::cli::vault::output::vaults_summary()` nach `"desktop"` |
| `daemon/mod.rs` | `mod ipc_vault_compare;` und `mod ipc_vault_compare_client;` (je mit `#[path = "os/shared/…"]`, nach `ipc_vault_client`); `pub(crate) use ipc_protocol::{CompareHead, CompareItemWire, CompareSideWire, ResolveFailure};` und `pub(crate) use ipc_vault_compare_client::{vault_check, vault_compare, vault_compare_text, vault_merge_apply, vault_resolve, CompareViewFull, ResolveResult};`, beide mit `#[allow(unused_imports)] // U26 (CLI), U29 (GUI), U31 (Android)` wie die bestehenden Tresor-Ausfuhren |
| `daemon/os/shared/ipc_protocol.rs` | `pub(crate) use self::vault::{…}` um `CompareHead, CompareItemWire, CompareSideWire, ResolveFailure` erweitert |

`INSTRUCTIONS`: vorher 1 969 Byte (1 967 Zeichen), erster Absatz 503 Byte; nachher 2 027 Byte (2 025 Zeichen), erster Absatz 503 Byte (unverändert). Der Test misst `str::len()`, also Byte; Grenzen 2 048 und 512 eingehalten (21 Byte Reserve gesamt). Beschreibung des Werkzeugs `se`: 1 309 Byte (Grenze 2 048).

Abweichungen: Die zwei `pub(crate) use` aus Punkt 4 tragen `#[allow(unused_imports)]`, weil GUI und Android sie noch nicht nutzen und die CLI nur `CompareSideWire`, `CompareItemWire`, `CompareViewFull` und die Client-Funktionen braucht. Kein Satz der `INSTRUCTIONS` musste gekürzt werden. `INSTRUCTIONS` nennt `screen_input` bereits in der Liste der nachzuladenden Werkzeuge (Stand REG-B ist insoweit erledigt).
