# Repository Instructions

## no local builds or test execution

- Codex must not run builds, compilers, linkers, test suites, packaging, release builds, or
  publication workflows on this workstation. This applies to the main agent and every subagent.
- In particular, do not invoke `cargo build`, `cargo check`, `cargo test`, `rustc`, native task-suite
  entrypoints that invoke Cargo, cross-compilers, installer/feed builders, or
  `native\publish-release-local.ps1` locally. Do not work around this rule with another local
  wrapper, linker, process, container, VM, WSL instance, or background agent.
- Local work is limited to repository inspection, source/documentation edits, static text/parsing
  checks that cannot invoke a compiler or linker, commits, pushes, and orchestration/inspection of
  remote CI. Stop any accidentally started local build immediately before continuing.
- The single task-level suite and the terminal release workflow must run only on the configured
  remote CI/automation runner after the candidate has been committed and pushed. The main agent
  owns defining the suite, triggering exactly one appropriate remote pipeline, monitoring it,
  evaluating its logs, and coordinating fixes; it does not execute the workload locally.
- These rules override any later wording that refers to running a suite, build, release wrapper, or
  validation command locally. Keep checked-in local entrypoints maintained for human or remote-runner
  use, but Codex must not invoke them on this workstation.

## subagent scope and RAM budget

- Keep Codex agent concurrency capped at eight threads per session. The main agent counts as one,
  so it may have at most seven direct children active at once. Prefer fewer whenever the same work
  can progress sequentially, and do not spawn while a task-level suite, build, package, or release
  process is running.
- Only the main agent may spawn agents. Subagents must never spawn sub-subagents or recursively fan
  out work.
- Give every subagent one bounded, coherent outcome. Do not combine broad repository exploration,
  design, implementation, verification, and release ownership in one assignment.
- Before spawning, the main agent must record in the brief:
  - the exact files, directories, symbols, or line ranges the subagent may read, avoiding broad
    repository-wide wildcards; keep that surface as small as the coherent outcome allows, but permit
    a larger connected surface when splitting it would hide a required integration boundary, and
    explain that exception in the brief;
  - the exact files it may create or modify;
  - the files, modules, and activities it must not touch;
  - the concrete deliverable, acceptance signal, and required report format.
- A subagent may inspect and edit only the explicitly assigned surface. It must not list or explore
  other folders, broaden its own scope, or ask another agent to fill a gap. It should make safe local
  decisions inside the assignment and report unresolved out-of-scope dependencies to the main
  agent.
- Subagents are zero-compute workers: they must not run builds, test suites, servers, package
  installs, long-running processes, commits, pushes, graph rebuilds, packaging, or release commands.
  The main agent owns suite definition, remote-CI orchestration and monitoring, integration, commits,
  pushes, and terminal release coordination, but must not execute build/test/release workloads locally.
- Every subagent result must list files read, files created or modified, key findings or changes,
  decisions made, and unresolved issues. When its assigned outcome is complete, it stops.

- Commit each coherent requested change or implementation milestone separately as soon as that milestone is complete. Several milestone commits from the same active task batch may be pushed together to the configured remote branch to avoid redundant CI builds; intermediate commits and pushes are development checkpoints, not releases.
- Unless the user explicitly says not to or pushing is technically blocked, do not leave completed work only in the worktree or local commits. Push the completed milestone commit(s), then report the branch, commit(s), and push result.
- A release is the single terminal distribution event after all user-requested work intended for that release is complete. Batch those open tasks together. Native source changes, intermediate commits, review fixes, test fixes, and untagged CI retries must not independently trigger a version bump, release-artifact build, tag, or GitHub Release.
- The required execution order is strict: implement **all** user-requested changes in the active batch first; create or update **one** automated task-level test suite that covers those changes and their directly affected integrations; commit and push the candidate; run that suite once through remote CI; evaluate its results and fix every discovered issue; rerun only that same remote suite when a fix requires confirmation; then perform one remote release and stop.
- Tests are verification overhead, never delivered work, milestones, or progress. Never report test counts or time spent testing as accomplishments.
- Never manually run broad test collections or hundreds of tests. Do not manually invoke workspace/all-target/all-feature test matrices, large unfiltered `cargo test` runs, or batches of individual test commands. Put the required coverage behind the single automated task-level suite and invoke that one suite as one command. A broader test run requires an explicit user request.
- Every test invocation must answer a specific question about the requested change or behavior directly affected by it. Never test merely because a test exists, to accumulate coverage, or to create the appearance of progress.
- During implementation, use only the smallest non-test feedback needed to keep editing safely, such as formatting, parsing, or a narrow compile/check of the changed component. Do not interrupt each small edit with a new validation cycle.
- If inspection, implementation, or the one automated suite exposes an obvious defect in the affected code, fix it immediately. Do not ignore a clear issue merely because the user did not name it separately, but do not expand into unrelated cleanup or additional test campaigns.
- "Small and verifiable" describes a coherent behavioral milestone, not a file count. A milestone is small when it has one bounded outcome, a known affected boundary, and a clear acceptance signal; it may require coordinated changes across multiple files or modules. Never classify work as small merely because its diff is confined to one file.
- Keep both large and small milestones in the implementation plan. Every milestone must define a concrete expected result that can be tested, but recording that result does not authorize a test run during implementation. Collect all milestone expectations into the one final automated task-level suite and verify them together in one invocation after every planned change is implemented.
- The automated task-level suite must use existing development/test binaries whenever possible. On its remote runner, if code changed, it may perform at most the incremental build needed for the affected component and must reuse that output throughout the fix loop. It must not invoke a full workspace, all-target, cross-platform, installer, feed, or release build. A full release-artifact build happens remotely once, only after the suite is complete.
- Before starting the complete release-artifact workflow, preflight the required Windows/WSL environment, Rust targets, `rustfmt`/`clippy`, Zig, NSIS, MinGW, network access, credentials, and publication permissions. Resolve missing setup before the full build so tool discovery does not cause repeated release builds.
- Artifact-mutating release paths share one cross-host, fail-fast lock at `release-native/.complete-release.lock`. Never remove that file merely because one PID is absent: Windows and WSL do not share a PID namespace. If a hard crash leaves it behind, first verify that no Windows, WSL, or Linux release process remains, inspect its owner metadata, and only then remove that one stale lock file.
- Enter the release stage only after the complete user-scoped task batch is implemented and the one automated task-level suite has been evaluated successfully. The version bump, one complete release-artifact build, artifact commit/push on `main`, matching `vX.Y.Z` tag, GitHub Release, and final publication check form one terminal release transaction performed exactly once at the end of that batch. Never run the complete release-artifact workflow merely to test or checkpoint an intermediate change. Never leave a successful complete release build unpublished unless publication is technically blocked.
- Use exactly one automated publication pipeline for an exact candidate. It consumes the artifacts from the single completed release build and publishes them; it must not rerun the task-level suite or rebuild the candidate. Never trigger `main`, `verify/v*`, tag, dispatch, and `release/v*` validation runs as competing or sequential duplicate pipelines for the same commit. A `verify/v*` run is allowed only when the user explicitly requests verification without a release. CI must never rebuild or rewrite committed release bytes.
- A failure before a tag is pushed stays in the same task batch and intended version: fix it, rerun only the same automated task-level suite or the failed release stage, and do not create another patch version or release. Do not invent additional gates, test matrices, verification branches, or publication attempts. If publication is externally blocked, preserve that candidate and report the blocker instead of starting another release.
- A complete release must have a GitHub Release on the repository's Releases page for the same `vX.Y.Z` as `native/Cargo.toml`, `release-native/update-feed/version.txt`, and the installer. Prefer the existing CI path: push the `vX.Y.Z` tag. If tag push is technically blocked, use the `workflow_dispatch` fallback with the required `publish_release=true` input, or the documented `docs/RELEASING.md` fallback of pushing `release/vX.Y.Z`. Do not call the release done until the CI release job has succeeded and the GitHub Release is visible with the expected assets: Windows installer, Windows app/updater/`se` payloads and hashes, Linux app/updater/`se` payloads and hashes, `install-linux.sh`, context-menu DLL, share-server payloads, and `version.txt`. If publishing is blocked, report the exact trigger attempted, the failure, and the remaining release step.
- `native\publish-release-local.ps1` remains the single stable entrypoint for a complete release, but Codex must not invoke it on this workstation. The configured remote automation runner must invoke this top-level script and let it orchestrate its required Windows and WSL/Linux builds, staging, hashes, feed promotion, version consistency, release commit/push, tag, GitHub publication, and final visibility check itself, calling checked-in subscripts internally where appropriate. Do not manually reproduce its steps, create a one-off release workflow, or change the release procedure from one release to the next. If the top-level script lacks a required release step, extend and stabilize it before remote use instead of performing that step manually. Do not treat `native\publish-update.ps1 -AllowPartialFeed` as a complete release unless the user explicitly asks for a Windows-only/partial feed.
- What can be reliably orchestrated by one checked-in script must be run by remote CI through that script rather than as a sequence of manual Codex commands. This applies both to the single task-level test suite and to the complete release. The script owns step ordering, subprocess handling, cleanup, and waiting on the remote runner.
- Any remote-CI job that runs an automated suite, build, cross-compile, packaging, release, publication, or release verification must have a timeout of at least 30 minutes. Give the remote invocation of `native\publish-release-local.ps1` at least two hours so the entire release can finish in one invocation. Do not use short/default timeouts and then extend them repeatedly. Poll the same remote job; never start a duplicate. If it times out, inspect its Windows and WSL child processes through runner diagnostics and wait for active build children to finish before retrying. A failed full wrapper preserves its isolated stage and prints the path. Treat that stage as recovery evidence, not as an automatically resumable candidate: the current wrappers have no general resume switch. Fix the cause and rerun the same remote wrapper for the same intended version; Cargo may safely reuse its own validated build cache, but do not hand-promote files from the retained stage.
- Let release jobs run unattended. Codex must check release/build-publication status no more often than once every 30 minutes; do not actively poll or emit repeated unchanged release-status messages between those checks. Work on other authorized tasks meanwhile. This restriction applies to agent monitoring, not the checked-in remote wrapper's own subprocess/publication coordination. Never start a duplicate release while one is running.
- For permitted GitHub Actions checks, use the GitHub API/connector directly. Do not park the shell in long `Start-Sleep` calls; use the product's wait/scheduling mechanism when otherwise idle. A timed-out local wait is not CI evidence.
- Do not assume the GitHub CLI is installed. Prefer the GitHub connector or REST API for release and workflow verification; if `gh` is used, check `Get-Command gh -ErrorAction SilentlyContinue` first and fall back immediately when it is missing.
- For multi-line PowerShell verification snippets, do not pipe a `foreach` statement block directly into `Format-*` or another command. Collect rows in an array (or wrap the producer in `& { ... }`) and then pipe the resulting objects, so verification commands fail only on real release issues rather than parser syntax.
- For WSL setup commands that need sudo, first run a non-interactive check such as `sudo -n true`. Do not run `sudo apt-get ...` in a Codex tool call if sudo would prompt for a password; it will hang without useful output. If a sudo/apt command times out, inspect and terminate only the stuck `sudo`/`apt`/`dpkg` process before continuing.
- If the local release script still encounters missing WSL, Rust targets, `rustfmt`/`clippy`, Zig, NSIS, or MinGW tooling despite the preflight, fix the local setup or script and rerun the same release wrapper for the same intended version. Use the preserved stage to identify the failed boundary, but do not claim automatic resume support, hand-copy release payloads, or recreate ad hoc linker wrappers as the final process.
- If tag-triggered CI fails after a tag was pushed, do not rewrite or move the published tag. First determine whether the exact tagged candidate can be retried unchanged (for example after transient infrastructure or publication failure); if so, retry publication for that same tag. Only when source or release artifacts must change and the immutable tag therefore cannot be reused may a new patch version be created. Treat that as exceptional recovery, complete all fixes and pre-publication gates before rebuilding, and report why the additional version was unavoidable.
- If CI fails at `cargo audit`, prefer a real dependency update first. If the advisory is blocked by a transitive crate pin, use only explicit `--ignore RUSTSEC-...` entries with comments naming the dependency path and removal condition; do not add broad audit suppression.
- Before triggering release publication with a tag, workflow dispatch, or `release/vX.Y.Z` branch, verify `native/Cargo.toml`, `release-native/update-feed/version.txt`, the matching installer, all six update-feed payloads, and their `.sha256` files agree for the same version.
- If a remote is missing, credentials fail, or the work is not safe to commit yet, state that clearly and explain what remains.

## mandatory workflow

For every requested change or investigation in this repository, use this two-stage planning and research flow before implementation:

1. Set the complete user-requested batch as the explicit goal and identify its concrete deliverables.
2. Analyze the relevant codebase, documentation, artifacts, graph, tests, and established patterns. From that evidence, create the stage-one implementation plan.
3. Research the approach broadly. Check established protocols, standards, libraries, platform conventions, and comparable implementation paths. Evaluate the planned approach for correctness, security, efficiency, reliability, and maintenance.
4. Write the stage-two detailed milestone plan. Keep every necessary large and small coherent behavioral milestone, the files or modules it affects where known, its dependencies, and a concrete testable expected result. A milestone may span multiple files; file count does not define its size. Define the acceptance signal now, but do not execute it yet.
5. Research plan gaps a second time. Resolve remaining API, protocol, platform, security, and implementation questions, then update the milestone plan into its final form before editing.
6. Implement all planned changes and fix obvious defects encountered in the affected code. Commit coherent implementation milestones, but do not turn each milestone into a test, build, push, or release cycle.
7. After all implementation is complete, create or update exactly one automated task-level suite that maps every planned milestone to its expected result and also covers their directly affected integrations. The suite must be self-contained, discover required runtime values through its own commands rather than undocumented prior knowledge, and accept or reuse development binaries without performing a full build.
8. Commit and push the candidate, then trigger the complete suite once through its single checked-in entrypoint on the configured remote CI runner with a timeout of at least 30 minutes so all milestone results are evaluated together. Fix any relevant implementation or suite issue it exposes, let remote CI incrementally rebuild only an affected component when its code changed, and rerun only that same remote entrypoint when confirmation is needed. Do not add per-milestone runs, manual test batches, unrelated validation matrices, repeated full builds, or any local build/test invocation.
9. When the implementation and remote suite are complete, trigger the configured remote release automation once so it invokes `native\publish-release-local.ps1` with a timeout of at least two hours. Monitor that one job through publication, report the result, and stop. Never invoke the release wrapper locally.

## documentation hygiene

These rules apply to first-party documentation (`README.md`, `native/README.md`, `DISCLAIMER.txt`, `AGENTS.md`, and `docs/**/*.md`) unless a task explicitly says otherwise.

- Treat `docs/TODO.md` as the only live board for open work. An item is open only when current code, artifacts, tests, or a real external blocker prove it is still open.
- Treat `docs/ROADMAP.md` as historical roadmap/status narrative, not as the current release source. Live version truth is `native/Cargo.toml` plus `release-native/update-feed/version.txt`.
- Treat `README.md` and `docs/RELEASING.md` as the canonical install/release documentation. When release scripts, feed layout, installer names, tags, update behavior, or supported artifacts change, update those docs in the same change.
- Treat `docs/SESSION_STATE.md`, `docs/*_research/**`, `docs/cfapi_review/**`, `docs/sync_research/**`, and `docs/vfs_research/**` as historical handoff/evidence unless explicitly refreshed. Do not use them as live state without checking current code and artifacts.
- Before editing docs, run a documentation context gate: `git status --short`, `git log -1 --oneline`, targeted `rg` for the topic, and any relevant code/artifact checks.
- For release/version claims, verify `native/Cargo.toml`, `release-native/update-feed/version.txt`, the matching installer, the matching `vX.Y.Z` tag, the matching GitHub Release, and all update-feed `.sha256` files.
- For code-behavior claims, verify current source with `rg` and, when useful, graphify before marking work shipped or open. Historical prototypes must be labeled historical/superseded when their source files no longer exist.
- For volatile external claims, check current primary sources and record the check date in the doc when the claim materially affects status or guidance.
- After documentation changes, run stale searches for old versions, `Current release`, `WIP`, `needs release`, `prefetch next`, contradictory status words, and old release commands.
- Documentation-only changes do not require native checks, a native patch bump, release build, release tag, or graphify rebuild. Native source changes require the native and graphify validation rules below, but do not independently trigger versioning or a release; release timing follows the single final release policy above.

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
- During native implementation, use only static editing feedback that cannot invoke Rust compilation or linking. Do not run `cargo fmt`, `cargo check`, `cargo test`, `cargo build`, `rustc`, or any native suite locally; exercise the requested behavior only through the one remote-CI task-level suite defined by the mandatory workflow.
- After modifying native source code, keep the graph current using the graphify commands in the `graphify` section below.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships. The initial graph is AST-only, built from `native/src` into the repository root.

When the user types `/graphify`, invoke the `skill` tool with `skill: "graphify"` before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying native source code, update the root graph from the repository root. If `graphify-out/manifest.json` already exists, `graphify extract native/src --out . --no-cluster` may incorrectly rewrite `graphify-out/graph.json` with only the changed files; do not leave that partial graph in place. For a reliable full root refresh, first verify the resolved targets are inside the repository-root `graphify-out/`, remove only the generated root files `graphify-out/manifest.json`, `graphify-out/graph.json`, and `graphify-out/GRAPH_REPORT.md`, then run `graphify extract native/src --out . --no-cluster` and `graphify cluster-only . --no-viz --no-label`.
- Do not use `graphify update native/src` for the required root graph refresh; it writes a separate `native/src/graphify-out/` instead of updating the repository-root graph. If that directory is accidentally created, verify it is ignored/untracked and remove only that generated `native/src/graphify-out/` directory after confirming the root `graphify-out/graph.json` has been rebuilt and clustered.
