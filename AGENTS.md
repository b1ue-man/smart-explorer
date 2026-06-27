# Repository Instructions

- After completing requested changes in this repository, commit the work and push it to the configured remote branch unless the user explicitly says not to or pushing is technically blocked.
- Do not leave completed work only in local commits; report the branch, commit, and push result.
- For native app changes, always bump the patch version, build the Windows release artifacts, commit the version/artifact changes on `main`, push `main`, create the matching `vX.Y.Z` tag, and push the tag unless the user explicitly says not to or this is technically blocked.
- For release work, always build the release artifacts before calling the release done, then commit and push the release changes and artifacts to the configured remote unless the user explicitly says not to or pushing is technically blocked.
- For a local Windows release, use `native\publish-release-local.ps1` as the default release command. It builds the Windows artifacts, refreshes the WSL/Linux feed payloads, writes the feed version last, and verifies all feed hashes. Do not treat `native\publish-update.ps1 -AllowPartialFeed` as a complete release unless the user explicitly asks for a Windows-only/partial feed.
- If the local release script fails because WSL, Rust targets, `rustfmt`/`clippy`, Zig, NSIS, or MinGW tooling is missing, fix the local setup or the script and rerun the release command. Do not hand-copy release payloads or recreate ad hoc linker wrappers as the final process.
- Before pushing a release tag, verify `native/Cargo.toml`, `release-native/update-feed/version.txt`, all four update-feed payloads, and their `.sha256` files agree for the same version.
- If a remote is missing, credentials fail, or the work is not safe to commit yet, state that clearly and explain what remains.

## native Rust architecture

These rules apply to `native/src` unless a task explicitly says otherwise.

- Keep files narrowly scoped to one feature responsibility. New or substantially edited Rust source files must stay under 500 lines and under 50 KiB. Existing oversized files are technical debt: do not add meaningful new code to them without extracting a cohesive submodule first, or state the exception clearly.
- Split by behavior, not by convenience. Prefer separate files for domain types, parsing/formatting, persistence, protocol/wire code, UI rendering, background orchestration, and platform adapters instead of a single feature catch-all file.
- Keep `core/` truly platform-independent. `core/` code must not import `std::os::windows`, `std::os::unix`, `windows`, `windows-sys`, `winreg`, shell/registry/clipboard APIs, platform path encoders, or target-specific process extensions. Avoid `#[cfg(windows)]`, `#[cfg(target_os = ...)]`, and `cfg!(windows)` in `core/` except for tests that assert portable behavior.
- Put platform behavior behind `os/`. Use `os/windows.rs`, `os/linux_os.rs`, and `os/shared/*` adapters selected from `mod.rs` with `#[cfg(...)]`/`#[path = ...]`, rather than scattering inline platform branches through `core/`. `os/shared` is for host-facing code that is genuinely portable across supported OSes; if it needs one OS crate or FFI call, move that part into the OS-specific adapter.
- Design the `core`/`os` boundary as a small typed API. `core` should own pure data models, planning, validation, parsing, and deterministic decisions. `os` should own filesystem quirks, shell integration, process launching, dialogs, credentials, registry/autostart, clipboard, platform metadata, and network mounts. Pass OS facts into `core` as typed values or traits instead of letting `core` discover the OS itself.
- Keep module public surfaces small. Re-export only the intended feature API from `mod.rs`; keep helpers private or `pub(crate)`. Prefer newtypes/enums/builders over raw strings, booleans, or loosely coupled tuples when they encode domain meaning.
- Treat recoverable failures as `Result`. Avoid `unwrap`, `expect`, and `panic!` in production paths unless they document a real invariant with a specific message. They are acceptable in tests and in one-time startup invariants where recovery is impossible.
- Keep dependencies platform-conscious. Put OS-specific crates under target-specific Cargo sections, keep default features off when they pull native TLS/crypto/toolchain dependencies, and document any native dependency or cross-compile risk in `Cargo.toml` or `docs/GOTCHAS.md`.
- For staged updates or elevated helper flows, bind every staged executable to an expected SHA-256 and revalidate it immediately before replacement or relaunch. Length checks alone are not sufficient.
- For sync/delete/overwrite flows, preserve retryability and reversibility as hard invariants. Failed apply steps must not be written into a new baseline as successful, and destructive changes must not proceed when the backup/conflict-copy step fails.
- Recursive delete code must never follow symlink, junction, or reparse-point children out of the authorized root. Validate every effective child target or treat link-like directories as non-recursive boundaries.
- `os/shared` must stay free of direct Windows/Unix imports, shell/process FFI, registry, reparse-point, platform metadata, and platform-specific `CommandExt` behavior. Put those behind per-OS adapter functions, even when the caller is already under `os/`.
- Any new CI/release guard added during a fix must be reflected in docs and scripts together, so local release, CI release, and auto-update feed behavior do not drift.
- Before finishing native source changes, run `cargo fmt` and the narrowest meaningful `cargo check`/`cargo test` from `native/`. For changes that touch shared APIs, run broader host checks as well. If checks are skipped, report why.
- After modifying native source code, keep the graph current using the graphify commands in the `graphify` section below.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships. The initial graph is AST-only, built from `native/src` into the repository root.

When the user types `/graphify`, invoke the `skill` tool with `skill: "graphify"` before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying native source code, run `graphify extract native/src --out . --no-cluster` and then `graphify cluster-only . --no-viz --no-label` to keep the root graph current (AST-only, no API cost).
