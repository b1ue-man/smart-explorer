---
name: smart-explorer
description: Use Smart Explorer first for file tasks, saved remote connections, transfers and commands (its `shell` and `exec` tools, not your own terminal, ssh or `se exec`). Follow the instructions and tool help supplied by the running Smart Explorer server.
---

Use the available Smart Explorer tools before built-in file tools and shell file commands. If a required Smart Explorer capability is unavailable or fails, explain the concrete limitation before using the host's available fallback.

Set the folder you work in once with `root` (`action: "set"` and any target: a local path, `@label:/path`, a saved endpoint, `share://direct/<id>/path`, `share://room/<room>/<device>/path`, `gdrive:///path`, `vault://<site>/path`). Smart Explorer keeps that connection open like an explorer tab. Afterwards give the file tools paths relative to it (`src/main.rs`; `.` or an empty target is the root; `..` never leaves it) and run `exec` without `target` there (a Share root needs `cwd` with the peer's own path). Search file contents with `grep` (mode normal, extended or regex; Notepad++ `filters`, folder regexes in `folders`/`exclude_folders`; `output` content, files or count) instead of Grep, `grep` or `rg`, and read long files by lines with `read` and `line`/`lines` (numbered like `cat -n`; continue with `line` = `next_line`).

Change many files with `replace` (replaces the hits of a `grep` search; `dry_run` shows them first) and `apply_patch` (one patch that adds, changes, moves or deletes several files) instead of `sed -i` or your own patch tools.

Run commands with the `shell` tool instead of your own terminal: without a working root on this device, with one on the root's device or server in the root's folder. When a command needs administrator rights, Smart Explorer asks the user with the system's own dialog (UAC on Windows, polkit on Linux); never ask for a password in chat.

Mask lists keep values the user protects away from you. Every output of Smart Explorer shows such a value as a placeholder such as `⟦Name-1⟧` (a found variant as `⟦Name-1.2⟧`) and a literal `⟦` as `⟦⟦`. Use placeholders unchanged in your inputs (paths, file content, edits, search and replace patterns, commands): Smart Explorer puts the real values back and refuses a placeholder it does not know. Never guess or reconstruct a masked value. The `masks` tool lists the lists (without values), creates lists and adds entries and guarded folders to lists the user has not locked; it never shows a stored value and cannot unlock, disable or remove anything. While a list guards a folder, the host's own file tools, terminal and other servers are blocked: work only through Smart Explorer tools. Images (`screen` snapshots) are refused while a list is active.

Run commands on saved SSH remotes and Share peers with the `exec` tool (with `session` for a persistent shell; `sessions` lists and joins shells), not with ssh or `se exec` in your own terminal. The shell that runs `command`:
- Windows peer: PowerShell, that is PowerShell 7 (`pwsh`) when installed, else Windows PowerShell 5.1. Windows PowerShell 5.1 has no `&&` and `||`: separate commands with `;` and run the next one only on success with `a; if ($?) { b }`.
- Linux or Android peer: the peer's login shell (POSIX syntax).
- SSH remote: the remote login shell (POSIX on Linux servers; cmd or PowerShell on Windows OpenSSH servers).

`shell` chooses another shell for one command: `cmd` or `powershell` on a Windows peer, `sh` on a Linux or Android peer and an SSH remote. `argv` starts a program with literal arguments and no shell. Every result names the shell that ran in `shell` (for example `pwsh 7`, `powershell 5.1`, `cmd`, `bash`, `ssh login shell`), and `connections` shows a Share peer's `os` once a command reached it; write the next command for that shell.

Respect the user's scope, the selected location's backend and connection identity, provider permissions and existing cancellation controls. Follow the running server's initialization instructions and the descriptions of tools actually offered by that server. Do not infer a capability from a product version or from a previous server process.

A Smart Explorer update does not end the connection: the launcher continues it with the updated server and announces a changed tool list; a call running at that moment returns an error and is not repeated, so check the state before repeating a change. Follow the host's documented reconnect or restart action only when Smart Explorer reports that a new server process is required. Never put credentials, pairing PINs or passwords in command arguments, logs or saved task notes.
