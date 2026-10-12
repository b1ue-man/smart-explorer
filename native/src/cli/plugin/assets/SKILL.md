---
name: smart-explorer
description: Use Smart Explorer (se) to work with the user's saved remote servers (SFTP, FTP/FTPS, WebDAV, SMB, Windows shares), Smart Explorer Share peers, Google Drive and local files — browse, read, edit, copy, move, search — and to run commands on saved SSH remotes or authorized Share peers. Use it whenever a task names a server, remote, peer, saved connection or another machine of the user.
---

# Smart Explorer

The `smart-explorer` MCP server (tools `connections`, `list`, `stat`, `read`,
`write`, `edit`, `mkdir`, `remove`, `copy`, `move`, `search`, `exec`,
`job_status`, `job_input`, `job_cancel`, `status`, `se`) uses the same saved
connections, credentials and background worker as the user's Smart Explorer
app and `se` terminal command.

## Workflow

1. Call `connections` first. Each entry has a `target` prefix and an `exec`
   kind: `ssh` (saved SFTP remote: commands run through the remote login shell)
   or `share` (Share peer: works once that device granted this one Exec).
2. Address files with targets: `@label:/path` (below the saved root),
   `share://direct/<id>/path`, `gdrive:///path`, or absolute local paths.
   Relative local paths are rejected.
3. Read before you change: `read`/`stat` return an `etag`; pass it as
   `if_match` to `write`/`edit` so you never overwrite someone else's change.
   `edit` replaces exact text and fails when `old_text` is missing or ambiguous.
4. `copy`, `move`, `search`, `exec` and `se` may outlive one call. When the
   answer has `status: running`, follow the `job_id` with `job_status`
   (`wait_seconds`, then `stdout_offset`/`stderr_offset`/`hits_offset` =
   the returned `next_offset`), send input with `job_input`, stop with
   `job_cancel`. `job_status` without `job_id` lists all jobs.
5. Large or binary files: `copy` them to a local path instead of `read`.

## Rules

- Do not ask the user for passwords in the chat. Saved credentials are used
  automatically. New logins: prefer SSH keys (`se` tool:
  `["connections","add","sftp","--host",…,"--key",…]`); for a password login
  ask the user to run `se connections add … --password-stdin` in a terminal.
- `remove`, `move`, overwriting `write`/`copy` and `exec` change remote systems
  without undo. Do them only as the task requires; confirm unclear deletions
  with the user.
- Change Share trust only on the user's explicit request: accepting access
  requests, enabling Exec grants, discoverable mode, adding peers or rooms.
- When something fails, call `status`; it shows the background worker, Share
  connectivity and credential store state.
- Any other Smart Explorer function is reachable through the `se` tool, e.g.
  `["share","status","--json"]`, `["drive","list"]`, `["update","--check","--json"]`;
  `["<command>","--help"]` explains each command.
