# RV1 – bestehende Namespace-Forwarder erhalten

Stand: 2026-10-04. Anschluss ausschließlich der in `ci-fourth-fixes.md`
geplanten neuen `confirm_namespace`-API. Der vierte Remote-Run ist die
Evidenz für den fehlenden Namespaceabschluss; kein neuer Review.

Vor Sourceedit frisch gelesen: eigener Planabschnitt und
[post-publication-namespace.md](../../../../refs/post-publication-namespace.md),
die tatsächlichen neuen Trait-/Dispatch-/Exportdefinitionen sowie die
vorhandenen `sync_filesystem`-Methoden der vier betroffenen Produktwrapper.
Die APIdefinitionen sind separat als `0703bb5c` committed. Ihr Default
und fehlende Extensions geben weiterhin `Ok(false)` zurück.

Die vier bestehenden Wrapper reichen ausschließlich tatsächliche
Bestätigung ihres Backendvertrags weiter:

- `CachingBackend` verwendet direkt seinen Livebackend. Kein Cachewert
  ersetzt einen Flush oder eine Namespacebestätigung.
- `GuardedBackend` verlangt die bestehende Schreibzulassung vor und nach
  dem inneren Aufruf. Eine entzogene Zulassung oder ein innerer Fehler
  wird weitergegeben und bestätigt keinen erfolgreichen Abschluss.
- `UnavailableBackend` löst wie bisher sein gespeichertes gepinntes
  Liveziel auf. Remote-Relativepfade bleiben Pfade dieses Ziels.
- `AgentBackend` nutzt seine vorhandene tatsächliche
  `sync_filesystem`-Query-/Fallbackkette. Fehlende Extensions eines
  Servicepeers bleiben unbestätigt; die vorhandenen Queryfristen und
  Fehlerbehandlungen bleiben bestehen.

Self-Review: Signaturen mit den frisch gespeicherten Definitionen
abgeglichen; der Diff enthält nur die vier additiven Hookmethoden.
Whole-filesystem-Flush, Deferred-Inhalte, Stage-Finish, Pfadumsetzung,
Locators, Permissions und tatsächliche Targetidentität bleiben erhalten.
Tree-sitter meldet keine Rust-Parsingfehler; alle vier Quellen bleiben
unter 500 Zeilen und 50 KiB. `git diff --check` ist fehlerfrei.
Keine lokalen Compiler, Builds, Formatter oder Tests ausgeführt.

Acceptance: Die bestehenden Android-/Agent-/Hostconsumer müssen in
derselben integrierten RV1-Suite die reale Bestätigung erreichen.
Fehlende Fähigkeit, Flushfehler oder Rechteentzug bleiben ohne
Erfolgsbasis. Die OS-Produzenten liegen bei V-NAMESPACE, der
Post-publish-Engineconsumer und vorhandene Faultfixtures bei E-ENGINE.
Keine offene Definition innerhalb dieser vier Forwarder; ihre
Laufzeitbestätigung bleibt ausdrücklich offen.

Gelesen: eigener Plan, Namespace-Ref, `vfs/core/extensions.rs`,
`vfs/core/extension_calls.rs`, `vfs/mod.rs`, die vier unten genannten
Wrapper sowie ihre tatsächlichen Namespace-/Filesystem-Callstellen.
Geändert: `vfs/core/cache_extensions.rs`, `share/core/fs_guard_extensions.rs`,
`daemon/os/shared/ipc_backend_extensions.rs`, `agent/core/extensions.rs`.
Erstellt: dieser Handoff. Keine anderen Produktdateien geändert.
