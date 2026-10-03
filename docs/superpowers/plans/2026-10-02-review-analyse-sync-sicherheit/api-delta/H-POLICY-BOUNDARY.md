# H-POLICY-BOUNDARY – API-Delta

Stand: 2026-10-03. Enger Architekturanschluss der bestehenden H-DISPATCH-Hostpolicy. Die Source-Änderung ist abgeschlossen; Ausführung wird durch die gemeinsame Remote-Suite belegt.

## Erhaltene Consumer-API

```rust
crate::share::fs::local_paths::secure_local_target(
    root: &str, rest: &[String],
) -> std::io::Result<String>;

crate::share::fs::local_paths::to_os_path(
    path: &str,
) -> std::path::PathBuf;

crate::share::fs::ensure_local_share_handle_allowed(
    handle: &crate::local_access::DirectoryHandle,
) -> std::io::Result<()>;
```

Sichtbarkeit bleibt Share-intern. `fs::local_paths` ist jetzt der Alias auf `share::fs_local_paths` mit Quelle `os/shared/fs_local_paths.rs`. Das alte nested `#[path = "fs_local_paths.rs"]` entfällt; übrige Consumer brauchen keine Pfadänderung. Der öffentliche Export-/Endpoint-/Wire-Vertrag und Persistenzformate ändern sich nicht.

## Interner Typ- und Modulpfad

`share::fs_policy::TargetPolicy` wird `share::fs_host_policy::TargetPolicy`. `new(access, allow_system_writes, local)`, `with_root(root)`, `visible(path, link)` sowie `read/write/destructive(path) -> io::Result<()>` und die bisherigen Share-internen Felder bleiben gleich. `ensure_handle_allowed(&DirectoryHandle)` liegt ebenfalls in `fs_host_policy`.

Eigene Consumer: `fs.rs`, `fs_guard_backend.rs`, `fs_guard_reports.rs`, `fs_guard_stream.rs`. Der Hauptagent hat den bestätigten außer-scope Import in `fs_guard_bulk.rs` selbst migriert; der Worker hat diese Datei nicht gelesen oder geändert.

`fs_policy` exportiert intern weiterhin die pure `private_name/private_path/system_write`-Klassifikation. Die bereits bestehenden pure Textfunktionen `normalized` und `within` sind jetzt Share-intern für die Hostpolicy und den Windowsadapter nutzbar. Sie entdecken keine Hostfakten.

Der destruktive Preflight ist ein getrenntes `share::fs_host_destructive::check(&TargetPolicy, &str) -> io::Result<()>`; es gibt keinen neuen öffentlichen Mutationseinstieg.

## Ausgewählte OS-Pfadgrenze

Nur `share/mod.rs` wählt Windows bzw. den bestehenden Nicht-Windows-/Unix-Zweig:

```rust
to_os_path(&str) -> PathBuf;
from_os_path(&Path) -> String;
policy_contains(root: &Path, path: &Path) -> bool;
canonical_contains(root: &Path, path: &Path) -> bool;
```

Windows-Policy vergleicht dieselben normalisierten Case-/UNC-/Verbatim-Textformen wie zuvor. Unix-Policy und beide kanonischen Zieladapter behalten den bisherigen Komponentenvergleich. Die getrennten Containment-Funktionen erhalten die vorher verschiedenen Grenzen; sie versprechen keine neue Hardlink- oder TOCTOU-Garantie. Der Test-Helfer `create_directory_link(&Path, &Path) -> io::Result<()>` enthält OS-spezifische Symlink-Erstellung und ist ausschließlich unter `cfg(test)` vorhanden.

## Fixture-/AcceptanceSelector-Delta

Die Funktionsnamen bleiben gleich. Falls die Hauptsuite vollqualifizierte Pfade verwendet, sind genau diese alten Hostpfade zu ersetzen:

| Alt | Neu |
|---|---|
| `share::fs_policy::task_tests::review_task_host_read_only_is_a_rights_error_for_normal_write_paths` | `share::fs_host_policy_task_tests::review_task_host_read_only_is_a_rights_error_for_normal_write_paths` |
| `share::fs_policy::task_tests::review_task_host_system_opt_in_never_opens_app_private_or_versions` | `share::fs_host_policy_task_tests::review_task_host_system_opt_in_never_opens_app_private_or_versions` |
| `share::fs::tests::local_target_stays_under_root` | `share::fs_host_policy_task_tests::local_target_stays_under_root` |
| `share::fs::tests::symlink_escape_is_blocked_when_supported` | `share::fs_host_policy_task_tests::symlink_escape_is_blocked_when_supported` |

Reine Namens-/Systemklassifikation bleibt unter `share::fs_policy::task_tests`, reine Traversalsyntax unter `share::fs::tests::split_clean_blocks_traversal`. Vorhandene H-DISPATCH-Guard-/Report-AcceptanceSelector bleiben erhalten.

## Inventar und Rest

Die exakten gelesenen, geänderten, erstellten und gelöschten Pfade samt eigenem Self-Review stehen vollständig in [abnahme/H-POLICY-BOUNDARY.md](../abnahme/H-POLICY-BOUNDARY.md). Es gibt keine Cargo-, öffentlichen Protocol- oder Consumer-Formatänderungen. Keine zusätzliche Scope-Freigabe erforderlich; Hauptagent übernimmt die bestehende finale Suite, Graph und Veröffentlichung.

