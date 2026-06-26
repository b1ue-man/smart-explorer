# Graph Report - .  (2026-06-26)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 178 nodes · 440 edges · 10 communities
- Extraction: 94% EXTRACTED · 6% INFERRED · 0% AMBIGUOUS · INFERRED: 25 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `16efc18d`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]
- [[_COMMUNITY_Community 5|Community 5]]
- [[_COMMUNITY_Community 6|Community 6]]
- [[_COMMUNITY_Community 7|Community 7]]
- [[_COMMUNITY_Community 9|Community 9]]

## God Nodes (most connected - your core abstractions)
1. `String` - 18 edges
2. `replace_file_with_staged()` - 13 edges
3. `replace_target_from_staged()` - 12 edges
4. `staged_sha256_from_path()` - 12 edges
5. `Result` - 12 edges
6. `Result` - 10 edges
7. `String` - 10 edges
8. `swap_in()` - 9 edges
9. `Feed` - 9 edges
10. `download_with_required_sha256()` - 9 edges

## Surprising Connections (you probably didn't know these)
- `main()` --calls--> `arg_value()`  [INFERRED]
  native/src/bin/smart_explorer_updater.rs → native/src/bin/smart_explorer_updater/args.rs
- `apply_update()` --calls--> `wait_for_pid_exit()`  [INFERRED]
  native/src/bin/smart_explorer_updater.rs → native/src/updater/os/linux_os.rs
- `apply_update()` --calls--> `relaunch_elevated()`  [INFERRED]
  native/src/bin/smart_explorer_updater.rs → native/src/bin/smart_explorer_updater/launch.rs
- `apply_update()` --calls--> `replace_target_from_staged()`  [INFERRED]
  native/src/bin/smart_explorer_updater.rs → native/src/bin/smart_explorer_updater/replace.rs
- `optional_sha256()` --calls--> `normalize_sha256()`  [INFERRED]
  native/src/bin/smart_explorer_updater/args.rs → native/src/bin/smart_explorer_updater/hash.rs

## Import Cycles
- 1-file cycle: `native/src/updater/os/linux_os.rs -> native/src/updater/os/linux_os.rs`
- 1-file cycle: `native/src/bin/smart_explorer_updater/args.rs -> native/src/bin/smart_explorer_updater/args.rs`
- 1-file cycle: `native/src/bin/smart_explorer_updater/hash.rs -> native/src/bin/smart_explorer_updater/hash.rs`
- 1-file cycle: `native/src/bin/smart_explorer_updater/launch.rs -> native/src/bin/smart_explorer_updater/launch.rs`
- 1-file cycle: `native/src/bin/smart_explorer_updater/replace.rs -> native/src/bin/smart_explorer_updater/replace.rs`
- 1-file cycle: `native/src/updater/core/core.rs -> native/src/updater/core/core.rs`
- 1-file cycle: `native/src/updater/os/windows.rs -> native/src/updater/os/windows.rs`
- 1-file cycle: `native/src/updater/os/shared/feed.rs -> native/src/updater/os/shared/feed.rs`

## Communities (10 total, 0 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.17
Nodes (30): Error, Feed, Option, Path, PathBuf, PayloadSpec, Result, String (+22 more)

### Community 1 - "Community 1"
Cohesion: 0.20
Nodes (22): Error, Option, Path, PathBuf, Result, String, Vec, app_release_asset_name() (+14 more)

### Community 2 - "Community 2"
Cohesion: 0.22
Nodes (22): Duration, Feed, Option, Path, PathBuf, PayloadSpec, Result, String (+14 more)

### Community 3 - "Community 3"
Cohesion: 0.22
Nodes (18): ApplyArgs, Option, Path, PathBuf, Result, String, Vec, args_with_hashes() (+10 more)

### Community 4 - "Community 4"
Cohesion: 0.27
Nodes (15): Into, Error, Option, Path, PathBuf, Result, Self, String (+7 more)

### Community 5 - "Community 5"
Cohesion: 0.32
Nodes (15): copy_file_checked(), is_newer(), parse_sha256_file(), parse_ver(), replace_file_with_staged(), sha256_file(), staged_payload_path(), staged_sha256_from_path() (+7 more)

### Community 6 - "Community 6"
Cohesion: 0.30
Nodes (13): Option, PathBuf, Result, Self, String, Vec, ApplyArgs, arg_value() (+5 more)

### Community 7 - "Community 7"
Cohesion: 0.42
Nodes (9): Path, PathBuf, Result, String, normalize_sha256(), sha256_file(), unique_temp_file(), verify_sha256() (+1 more)

### Community 9 - "Community 9"
Cohesion: 0.33
Nodes (5): apply_update(), main(), ApplyArgs, Result, String

## Knowledge Gaps
- **13 isolated node(s):** `ApplyArgs`, `Result`, `String`, `Self`, `Vec` (+8 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `Duration` connect `Community 2` to `Community 0`, `Community 9`, `Community 1`?**
  _High betweenness centrality (0.402) - this node is a cross-community bridge._
- **Why does `apply_update()` connect `Community 9` to `Community 2`, `Community 3`, `Community 4`?**
  _High betweenness centrality (0.366) - this node is a cross-community bridge._
- **Why does `main()` connect `Community 9` to `Community 6`?**
  _High betweenness centrality (0.231) - this node is a cross-community bridge._
- **Are the 6 inferred relationships involving `replace_file_with_staged()` (e.g. with `copy_with_retries()` and `run_apply_worker()`) actually correct?**
  _`replace_file_with_staged()` has 6 INFERRED edges - model-reasoned connections that need verification._
- **Are the 8 inferred relationships involving `staged_sha256_from_path()` (e.g. with `apply_via_installed_updater()` and `ensure_installed_updater()`) actually correct?**
  _`staged_sha256_from_path()` has 8 INFERRED edges - model-reasoned connections that need verification._
- **What connects `ApplyArgs`, `Result`, `String` to the rest of the system?**
  _13 weakly-connected nodes found - possible documentation gaps or missing edges._