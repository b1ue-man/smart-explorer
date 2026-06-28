# Repository Instructions

- After completing requested changes in this repository, commit the work and push it to the configured remote branch unless the user explicitly says not to or pushing is technically blocked.
- Do not leave completed work only in local commits; report the branch, commit, and push result.
- For native app changes, always bump the patch version, build the Windows release artifacts, commit the version/artifact changes on `main`, push `main`, create the matching `vX.Y.Z` tag, and push the tag unless the user explicitly says not to or this is technically blocked.
- For release work, always build the release artifacts before calling the release done, then commit and push the release changes and artifacts to the configured remote unless the user explicitly says not to or pushing is technically blocked.
- For a local Windows release, use `native\publish-release-local.ps1` as the default release command. It builds the Windows artifacts, refreshes the WSL/Linux feed payloads, writes the feed version last, and verifies all feed hashes. Do not treat `native\publish-update.ps1 -AllowPartialFeed` as a complete release unless the user explicitly asks for a Windows-only/partial feed.
- If the local release script fails because WSL, Rust targets, `rustfmt`/`clippy`, Zig, NSIS, or MinGW tooling is missing, fix the local setup or the script and rerun the release command. Do not hand-copy release payloads or recreate ad hoc linker wrappers as the final process.
- Before pushing a release tag, verify `native/Cargo.toml`, `release-native/update-feed/version.txt`, all four update-feed payloads, and their `.sha256` files agree for the same version.
- If a remote is missing, credentials fail, or the work is not safe to commit yet, state that clearly and explain what remains.

## mandatory workflow

For every requested change or investigation in this repository, follow this flow by default without waiting for the user to ask for planning or research. Do not narrate the full checklist unless it helps the work; execute it as the normal operating loop.

1. Set the goal explicitly. Write the concrete objective in working notes or the task plan before editing, and do not change it unless the user changes the request or evidence shows the current goal is unsafe or wrong.
2. Analyze the codebase. Inspect the current source, docs, artifacts, graph, tests, and existing patterns needed to decide where and how the goal can be implemented. From that analysis, create an initial implementation plan.
3. Research the approach. Check whether the goal has established implementation paths, protocols, standards, libraries, or domain conventions. For every implementation-plan item that creates something new, verify whether it is sensible and whether a closer established protocol, library, or pattern exists. Evaluate the chosen path for security, efficiency, and reliability before committing to it.
4. Write a detailed milestone plan. Break the work into small, testable, verifiable intermediate results. Where it can be known safely, state which file or module each milestone affects and what belongs there functionally; do not prewrite the exact content unless needed. Each milestone must include its validation signal.
5. Research gaps a second time. Resolve open plan questions, choose any libraries or protocols to use, define the relevant APIs, commands, or calls, and update the milestone plan before implementation.
6. Implement by milestones. Treat each intermediate result as a milestone that must be completed and verified before later work builds on it. After each milestone, evaluate whether the implementation plan must be adjusted and revalidated. Any new change introduced by that adjustment goes through the same analysis, research, planning, and verification loop at the smallest practical scope.

## documentation hygiene

These rules apply to first-party documentation (`README.md`, `native/README.md`, `DISCLAIMER.txt`, `AGENTS.md`, and `docs/**/*.md`) unless a task explicitly says otherwise.

- Treat `docs/TODO.md` as the only live board for open work. An item is open only when current code, artifacts, tests, or a real external blocker prove it is still open.
- Treat `docs/ROADMAP.md` as historical roadmap/status narrative, not as the current release source. Live version truth is `native/Cargo.toml` plus `release-native/update-feed/version.txt`.
- Treat `README.md` and `docs/RELEASING.md` as the canonical install/release documentation. When release scripts, feed layout, installer names, tags, update behavior, or supported artifacts change, update those docs in the same change.
- Treat `docs/SESSION_STATE.md`, `docs/*_research/**`, `docs/cfapi_review/**`, `docs/sync_research/**`, and `docs/vfs_research/**` as historical handoff/evidence unless explicitly refreshed. Do not use them as live state without checking current code and artifacts.
- Before editing docs, run a documentation context gate: `git status --short`, `git log -1 --oneline`, targeted `rg` for the topic, and any relevant code/artifact checks.
- For release/version claims, verify `native/Cargo.toml`, `release-native/update-feed/version.txt`, the matching installer, the matching `vX.Y.Z` tag, and all update-feed `.sha256` files.
- For code-behavior claims, verify current source with `rg` and, when useful, graphify before marking work shipped or open. Historical prototypes must be labeled historical/superseded when their source files no longer exist.
- For volatile external claims, check current primary sources and record the check date in the doc when the claim materially affects status or guidance.
- After documentation changes, run stale searches for old versions, `Current release`, `WIP`, `needs release`, `prefetch next`, contradictory status words, and old release commands.
- Documentation-only changes do not require a native patch bump, release build, release tag, or graphify rebuild. Native source changes still follow the native and graphify rules below.

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
