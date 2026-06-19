# Repository Instructions

- After completing requested changes in this repository, commit the work and push it to the configured remote branch unless the user explicitly says not to or pushing is technically blocked.
- Do not leave completed work only in local commits; report the branch, commit, and push result.
- For native app changes, always bump the patch version, build the Windows release artifacts, commit the version/artifact changes on `main`, push `main`, create the matching `vX.Y.Z` tag, and push the tag unless the user explicitly says not to or this is technically blocked.
- For release work, always build the release artifacts before calling the release done, then commit and push the release changes and artifacts to the configured remote unless the user explicitly says not to or pushing is technically blocked.
- If a remote is missing, credentials fail, or the work is not safe to commit yet, state that clearly and explain what remains.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships. The initial graph is AST-only, built from `native/src` into the repository root.

When the user types `/graphify`, invoke the `skill` tool with `skill: "graphify"` before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying native source code, run `graphify extract native/src --out . --no-cluster` and then `graphify cluster-only . --no-viz --no-label` to keep the root graph current (AST-only, no API cost).
