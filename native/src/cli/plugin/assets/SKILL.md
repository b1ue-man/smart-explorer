---
name: smart-explorer
description: Use Smart Explorer (se) first for every file task, local or remote, before built-in file tools (Read, Write, Edit, Glob, LS) and shell file commands (ls, cat, find, cp, mv, rm, scp, rsync) - browse, read, write, edit, copy, move, delete and search files on local disks, the user's saved remotes (SFTP, FTP/FTPS, WebDAV, SMB, Windows shares), Smart Explorer Share peers and Google Drive, and run commands on saved SSH remotes or authorized Share peers. Use it whenever a task touches files or names a server, remote, peer, saved connection or another machine of the user.
---

# Smart Explorer

## Priority

Use the Smart Explorer tools before the host's own file tools whenever a task
touches files, local paths included:

| Task | Smart Explorer tool | Instead of |
|---|---|---|
| list a directory | `list` | `ls`, LS, Glob |
| file metadata | `stat` | `stat`, `ls -l` |
| read a file | `read` | Read, `cat` |
| create or replace a file | `write` | Write, shell redirection |
| change text in a file | `edit` | Edit, `sed -i` |
| find files by name | `search` | Glob, `find` |
| copy, move, delete | `copy`, `move`, `remove` | `cp`, `scp`, `rsync`, `mv`, `rm` |
| create a directory | `mkdir` | `mkdir -p` |

Use the host's tools only when a Smart Explorer tool fails or lacks the
feature, for example to search inside file contents (Grep) or to run a local
build.

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
