//! The tools `se mcp` offers. Names stay short: hosts prefix the server name
//! (Claude Code: `mcp__plugin_smart-explorer_se__list`, Codex:
//! `smart-explorer.list`). Annotations follow what each tool can change:
//! Codex asks for approval unless a tool is read-only
//! (docs/refs/codex-mcp-spec-rmcp-2026-10-12.md §1). The file tools, the
//! command tools (shell, exec, sessions, job_input, job_cancel) and `masks`
//! load upfront in Claude Code, and the instructions put them before the
//! host's own file tools and terminal
//! (docs/refs/claude-code-plugins-mcp-2026-10-12.md §7).
use rmcp::model::{JsonObject, MetaObject, Tool, ToolAnnotations};
use serde_json::{json, Value};

use super::screen_tool;

/// Server instructions. Codex asks to keep the first 512 characters
/// self-contained; Claude Code shows at most 2 048.
pub(super) const INSTRUCTIONS: &str = "\
Smart Explorer (se) is the user's file explorer. Use its tools first for every \
file task, local or remote (root, list, stat, read, write, edit, apply_patch, \
replace, mkdir, remove, copy, move, search, grep), before built-in file tools \
(Read, Write, Edit, Glob, Grep, LS) and shell file commands (ls, cat, find, \
grep, cp, mv, rm, sed); use those only when a Smart Explorer tool fails or \
lacks the feature.

Run commands with `shell`, not your own terminal: locally, or on the working \
root's device or server in its folder; elevation asks the user. `exec` runs on \
a named SSH remote or Share peer, not ssh or `se exec`. Windows peers run \
`command` in PowerShell: pwsh 7 if installed, else Windows PowerShell 5.1, \
which has no && or || (write `a; if ($?) { b }`). Linux/Android peers and SSH \
remotes use the login shell; exec's `shell` (powershell, cmd, sh) overrides \
it, `argv` runs a program without a shell, results name their `shell`. \
`sessions` lists and joins persistent shells. Long work returns a job_id: \
job_status, job_input, job_cancel.

Mask lists: outputs show protected values as placeholders like ⟦Name-1⟧, a \
literal ⟦ as ⟦⟦. Use them unchanged in inputs (paths, content, patterns, \
commands); se restores the real values. `masks` adds lists and entries, never \
shows values. While a list guards a folder, use only se tools.

Targets: absolute local paths, `@label:/path`, saved endpoints, `share://…`, \
`gdrive:///…`, `vault://…`. `root` sets a working root, kept open like a tab: \
paths may then be relative (`..` never leaves it); `shell` and `exec` without \
target run there. `grep` searches contents, `replace` changes hits (`dry_run` \
previews), `apply_patch` edits files; `read` takes `line`/`lines`; `etag` \
guards write/edit. Never ask for passwords in chat. Change Share trust (access \
requests, connection rights: `share grants confirm|set`, every `--preset`, Exec \
grants, program lists, device/room settings, discoverability) only when the \
user explicitly asks. The `se` tool runs every other se command.";

/// Tools Claude Code loads upfront (`_meta["anthropic/alwaysLoad"]`), so the
/// file workflow, local and remote commands and the mask lists are visible
/// without a tool search (agents that never found `exec` ran `se exec` and ssh
/// in their terminal; while a mask list guards a folder the host's own tools
/// are blocked, plan 2026-11-28 S1); the others stay deferred.
const UPFRONT: &[&str] = &[
    "connections",
    "root",
    "list",
    "stat",
    "read",
    "write",
    "edit",
    "apply_patch",
    "mkdir",
    "remove",
    "copy",
    "move",
    "search",
    "grep",
    "replace",
    "shell",
    "exec",
    "job_status",
    "job_input",
    "job_cancel",
    "sessions",
    "masks",
];

const TARGET: &str = "Smart Explorer target: an absolute local path, `@label:/path` of a saved remote, a saved `sftp://`, `ftp://`, `ftps://` or `webdav://` endpoint (`scheme://user@host:port/path`), `share://direct/<id>/path`, `share://room/<room>/<device>/path`, `gdrive:///path` or `vault://<site>/path`; with a working root (`root`) also a path relative to it (`.` or empty = the root, `..` stays inside). `connections` lists what exists.";
const WAIT: &str = "Seconds to wait for completion before returning a job_id (default 30). Keep it below your client's tool timeout (Codex: 60 s by default).";
const MAX_BYTES: &str = "Payload budget of this answer in bytes (default 20000). Agent hosts cut results above about 10000 tokens.";

pub(super) fn tools() -> Vec<Tool> {
    let tools = vec![
        tool("status", "Smart Explorer status",
            "Version, installation and app-data paths, credential store, background worker (login start, pause, check interval, automatic pause), number of sync setups, Share worker and connectivity state, a running Direct-code renewal with the devices it still waits for (`share.code_renewal`, null without one). Use it when a connection, peer, sync or remote command does not work.",
            json!({}), &[], read_only(false)),
        tool("connections", "Saved remotes and Share peers",
            "Every saved remote and Share peer/room with the target prefix to use, its live status, and how commands run there: `exec: \"ssh\"` for saved SFTP remotes (the remote login shell), `exec: \"share\"` for Share peers (works once that device granted this one Exec). A Share peer's `os` (windows, linux, android; null until an `exec` reached it) tells the shell of `exec`: PowerShell on Windows, the login shell on Linux and Android.",
            json!({}), &[], read_only(false)),
        tool("root", "Working root", super::root_tool::DESCRIPTION, super::root_tool::properties(), &[], read_only(true)),
        tool("list", "List a directory",
            "Directory entries (name, type file|dir|symlink|special, size, mtime_ms) sorted by name, one page per call; continue with `offset` = `next_offset`. A symlink's size is its own (the length of the path it holds). Use it instead of `ls`, LS or Glob, for local and remote directories.",
            json!({
                "target": {"type": "string", "description": TARGET},
                "offset": {"type": "integer", "minimum": 0, "description": "Index of the first entry (default 0)."},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &[], read_only(true)),
        tool("stat", "File or directory metadata",
            "Type, size, mtime_ms, provider id/md5 when known, and the `etag` that write/edit accept as `if_match`. Use it instead of `stat` or `ls -l`.",
            json!({"target": {"type": "string", "description": TARGET}}), &[], read_only(true)),
        tool("read", "Read a file",
            "File content from a byte `offset`, as UTF-8 text (default) or base64; or with `line` (first line, from 1) and/or `lines` (how many) numbered lines as `cat -n` prints them (number, tab, text; CR LF shown as a line end, `crlf` true), continued with `line` = `next_line` (null at the end; `total_lines` once the end was read). Binary files are not dumped as text: request `encoding: \"base64\"` or copy them to a local path. Returns `etag`, `size` and `next_offset` (null at the end). Through a link it reads the file the link points to; `size` and `etag` are then null. With an active mask list the whole file is masked before it is paged (`masked` true): `size`, `offset` and `next_offset` count bytes of the masked text, line numbers stay those of the file. Use it instead of Read or `cat`, for local and remote files.",
            json!({
                "target": {"type": "string", "description": TARGET},
                "line": {"type": "integer", "minimum": 1, "description": "First line to read (1-based); numbered output."},
                "lines": {"type": "integer", "minimum": 1, "description": "Number of lines (default: as many as fit the answer)."},
                "offset": {"type": "integer", "minimum": 0, "description": "First byte (default 0)."},
                "length": {"type": "integer", "minimum": 1, "description": "Bytes to read (default: the answer budget)."},
                "encoding": {"type": "string", "enum": ["text", "base64"], "description": "text (default) or base64."},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &["target"], read_only(true)),
        tool("write", "Write a file",
            "Creates a file, or replaces it with `overwrite: true`; missing parent directories are created. The content is staged next to the destination and published only when complete. `if_match` (an etag from stat/read) refuses the write if the file changed meanwhile. For large files write a local file and use `copy`. Use it instead of Write or shell redirection.",
            json!({
                "target": {"type": "string", "description": TARGET},
                "content": {"type": "string", "description": "New file content."},
                "encoding": {"type": "string", "enum": ["text", "base64"], "description": "How `content` is encoded (default text = UTF-8)."},
                "overwrite": {"type": "boolean", "description": "Replace an existing file (default false)."},
                "if_match": {"type": "string", "description": "Only write when the current file still has this etag."}
            }), &["target", "content"], changes(true, false)),
        tool("edit", "Edit a text file",
            "Applies exact text replacements to a UTF-8 file in order and saves it the same way as `write`. Each `old_text` must occur exactly once unless `replace_all` is set. Pass the etag of your last read as `if_match` to avoid overwriting someone else's change. Use it instead of Edit or `sed -i`.",
            json!({
                "target": {"type": "string", "description": TARGET},
                "edits": {"type": "array", "minItems": 1, "items": {
                    "type": "object",
                    "properties": {
                        "old_text": {"type": "string"},
                        "new_text": {"type": "string"},
                        "replace_all": {"type": "boolean"}
                    },
                    "required": ["old_text", "new_text"],
                    "additionalProperties": false
                }},
                "if_match": {"type": "string", "description": "Only save when the file still has this etag."}
            }), &["target", "edits"], changes(true, false)),
        tool("apply_patch", "Apply a patch to files", super::patch_tool::DESCRIPTION, super::patch_tool::properties(), &[], changes(true, false)),
        tool("mkdir", "Create a directory",
            "Creates the directory and missing parents; an existing directory is fine. Use it instead of `mkdir -p`.",
            json!({"target": {"type": "string", "description": TARGET}}), &["target"], changes(false, true)),
        tool("remove", "Remove a file or directory",
            "Deletes a file, or a directory with `recursive: true` (links inside are removed, never followed). Filesystem roots and a saved connection's configured root need `allow_root: true`. Like `se rm --force`; there is no undo. Use it instead of `rm`.",
            json!({
                "target": {"type": "string", "description": TARGET},
                "recursive": {"type": "boolean", "description": "Required for directories."},
                "allow_root": {"type": "boolean", "description": "Allow removing a filesystem root or a saved connection's root."}
            }), &["target"], changes(true, true)),
        transfer_tool("copy", "Copy files", "Copies between any two targets (local, saved remotes, Share peers, Drive) with `se cp` semantics: an existing destination directory receives the source inside it; directories need `recursive`; existing files are replaced only with `overwrite`. A file is published only when it is complete, so a failed source never leaves a partial file; a new destination is copied on the transfer engine (parallel, large files in ranges) and a failing file does not stop the other files. `progress` carries `rate_bps`, `limiter` (none, ramp, idle, source, target, remote, link, se_cpu, se_thread, memory), `limiter_text`, `segments` and `note` (a hint about the connection, such as how many SSH connections the copy uses). A link named as `source` or `destination` is followed (directory links only on local paths); links and special files inside a copied directory are skipped and listed in `omitted`. Use it instead of `cp`, `scp` or `rsync`."),
        transfer_tool("move", "Move or rename", "Moves with `se mv` semantics: a rename inside one connection when possible, otherwise copy, then remove exactly the copied source. A cancellation after the copy keeps the source. A link moves as itself (rename within its connection only). Links and special files inside a copied directory stay in the source (`omitted`), and so do the directories holding them (`kept_directories`). Use it instead of `mv`."),
        tool("search", "Search names", super::search_tool::DESCRIPTION, super::search_tool::properties(TARGET, WAIT, MAX_BYTES), &["query"], read_only(true)),
        tool("grep", "Search file contents", super::grep_tool::DESCRIPTION, super::grep_tool::properties(TARGET, WAIT, MAX_BYTES), &["pattern"], read_only(true)),
        tool("replace", "Replace in files", super::replace_tool::DESCRIPTION, super::replace_tool::properties(), &[], changes(true, false)),
        tool("shell", "Run a command here or at the working root", super::shell_tool::DESCRIPTION, super::shell_tool::properties(), &[], changes(true, false)),
        tool("exec", "Run a remote command",
            "Runs a command on a saved SFTP remote over its SSH login (`@label`, `@label:/dir` = working directory, or its saved sftp:// endpoint) or on a Share peer that granted this device Exec (peer name, id or share:// endpoint); without `target` on the working root's SSH remote or Share peer, in the root's folder (a relative `cwd` is below it; on a Share peer an absolute `cwd` is the peer's own disk path). `elevated`: as admin/root (peer: its user confirms; SSH: sudo, password asked locally). The tool `shell` runs commands locally or at the working root. Use it instead of ssh or `se exec` in a terminal. `command` is one shell string: a Windows peer runs it in PowerShell (pwsh 7 when installed, else Windows PowerShell 5.1, which has no `&&`/`||`: write `a; if ($?) { b }`), a Linux or Android peer in its login shell, an SSH remote in the remote login shell (cmd or PowerShell on Windows OpenSSH). `shell` overrides that for one command: `cmd` or `powershell` on Windows peers, `sh` on Linux/Android peers and SSH remotes. `argv` (program and literal arguments) starts a program without a shell. The result's `shell` names the shell that ran (`pwsh 7`, `powershell 5.1`, `cmd`, a login shell such as `bash`, `ssh login shell`, `none` for argv); a PowerShell parse error has line and column. `connections` shows each peer's `os`. No time or output limit unless set; output is kept completely and paged. Exit status, signal and timing are reported; job_input sends input, job_cancel cancels. With `session` (a name) the command runs in a persistent shell instead: the first call opens it on `target` (PowerShell on Windows peers, bash or sh elsewhere), later calls keep its working directory, environment, variables and functions (no PTY; the result adds `cwd`, `session_ended`). The user can join it with `se shell --attach` unless `attachable` is false; `sessions` lists and closes sessions and joins the user's command shells (`se shell -T --attachable`); `se shell` terminal sessions cannot be joined. Cancel or timeout end the whole session.",
            json!({
                "target": {"type": "string", "description": "Share peer selector or share:// endpoint, or a saved SFTP remote as @label[:/dir] or its sftp:// endpoint. Default: the working root's remote or peer; required without one unless `session` names an open session."},
                "command": {"type": "string", "description": "One command string for the target's shell: PowerShell on Windows peers (pwsh 7, else 5.1 without && and ||), the login shell on Linux/Android peers and SSH remotes."},
                "shell": {"type": "string", "enum": ["powershell", "cmd", "sh"], "description": "Shell for `command` instead of the target's own: powershell or cmd on Windows peers, sh on Linux/Android peers and SSH remotes. A session accepts only the shell it runs; cmd needs a command without `session`."},
                "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Program and literal arguments, instead of `command`."},
                "cwd": {"type": "string", "description": "Working directory (default: the working root's folder, else the remote user's home)."},
                "elevated": {"type": "boolean", "description": "Run as administrator/root: the peer's user confirms (UAC or polkit); SSH remotes use sudo, a password is asked locally."},
                "env": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Extra environment variables."},
                "stdin": {"type": "string", "description": "Text written to the command's standard input, followed by end-of-input unless `keep_stdin_open`."},
                "keep_stdin_open": {"type": "boolean", "description": "Leave standard input open for job_input."},
                "timeout_seconds": {"type": "integer", "minimum": 1, "description": "Stop the command after this many seconds (default: no limit); in a session this ends the session."},
                "session": {"type": "string", "description": "Persistent shell session name (letters, digits, '-', '_', '.'): opened on `target` by the first call, reused by later ones; also a session joined with `sessions`. Input goes as `stdin` (keep_stdin_open is not available)."},
                "attachable": {"type": "boolean", "description": "Only when the call opens a session: let the user's `se shell --attach` and other agents join it (default true)."},
                "wait_seconds": {"type": "integer", "minimum": 0, "description": WAIT},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &[], changes(true, false)),
        tool("job_status", "Follow jobs",
            "Without job_id: every job of this session. With job_id: state, progress, result and the next page of each output stream from the given offsets; `wait_seconds` waits for new output or the end first.",
            json!({
                "job_id": {"type": "string"},
                "wait_seconds": {"type": "integer", "minimum": 0, "description": "Wait up to this long for the job to end (default 0)."},
                "stdout_offset": {"type": "integer", "minimum": 0},
                "stderr_offset": {"type": "integer", "minimum": 0},
                "hits_offset": {"type": "integer", "minimum": 0},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &[], read_only(true)),
        tool("job_input", "Send input to a job",
            "Writes text to the standard input of a running `exec` or `se` job; `eof: true` closes it.",
            json!({
                "job_id": {"type": "string"},
                "data": {"type": "string"},
                "eof": {"type": "boolean"}
            }), &["job_id"], changes(false, false)),
        tool("job_cancel", "Cancel a job",
            "Stops a running job: copies stop at the next block and remove their unfinished stage file, SSH commands get TERM then KILL after 10 s, Share commands end their contained process tree. Waits up to `wait_seconds` for the end.",
            json!({
                "job_id": {"type": "string"},
                "wait_seconds": {"type": "integer", "minimum": 0, "description": WAIT}
            }), &["job_id"], changes(false, true)),
        tool("se", "Run an se command",
            "Runs any other Smart Explorer terminal command as `se <args>` and returns its exit code and output, for example: [\"share\",\"status\",\"--json\"], [\"share\",\"request\",\"list\",\"--json\"], [\"share\",\"grants\",\"--json\"], [\"share\",\"grants\",\"exec\",\"--json\"], [\"share\",\"exec\",\"log\",\"--json\"], [\"share\",\"exec\",\"approvals\",\"--json\"], [\"share\",\"exec\",\"accounts\",\"--json\"], [\"share\",\"device\",\"list\",\"--json\"], [\"share\",\"room\",\"list\",\"--json\"], [\"screen\",\"bench\",\"<device>\",\"--json\"], [\"screen\",\"codec\",\"--json\"], [\"screen\",\"windows\",\"<device>\",\"--json\"], [\"connections\",\"add\",\"sftp\",\"--host\",\"h\",\"--user\",\"u\",\"--key\",\"/home/u/.ssh/id_ed25519\",\"--label\",\"prod\"], [\"connections\",\"add-peer\",\"--code\",\"SE-D3-…\"], [\"drive\",\"mount\",\"@prod:/srv\",\"--letter\",\"M\"] (Windows), [\"drive\",\"mount\",\"@prod:/srv\",\"--mount-point\",\"/home/u/SmartExplorer/prod\"] (Linux), [\"drive\",\"devices\",\"status\",\"--json\"], [\"drive\",\"open\",\"<id>\"], [\"update\",\"--check\",\"--json\"], [\"doctor\",\"--json\"], [\"doctor\",\"--clear-hash-cache\",\"--json\"], [\"sync\",\"list\",\"--json\"], [\"sync\",\"compare\",\"Docs\",\"--json\"], [\"ls\",\"@prod:/srv\",\"--recursive\",\"--json\"], [\"analyze\",\"@prod:/srv\",\"--sort\",\"allocated\",\"--files\",\"20\",\"--types\",\"--json\"] (storage analysis: size and occupied space per entry, largest files, totals per type), [\"sync\",\"edit\",\"Docs\",\"--trigger\",\"realtime\"], [\"background\",\"status\",\"--json\"], [\"desktop\",\"status\",\"--json\"], [\"desktop\",\"enable\",\"--json\"], [\"vault\",\"list\",\"--json\"], [\"connections\",\"gdrive\",\"status\",\"--json\"]. `[\"--help\"]` or [\"<command>\",\"--help\"] explains every command. Prefer the dedicated tools for files and remote commands.",
            json!({
                "args": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Arguments after `se`."},
                "stdin": {"type": "string", "description": "Standard input (closed afterwards), e.g. for --pin-stdin."},
                "wait_seconds": {"type": "integer", "minimum": 0, "description": WAIT},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &["args"], changes(true, false)),
        tool("sessions", "Persistent shell sessions",
            "Lists this server's persistent shell sessions (`exec` with `session`) and the user's other attachable sessions (`se shell -T --attachable` command shells, other agents; `se shell` terminal sessions are listed with kind terminal and cannot be joined). Each row names its `dialect` (powershell or posix) and, once known, its `shell` (`pwsh 7`, `powershell 5.1`, `bash`, `sh`): Windows PowerShell 5.1 has no `&&`/`||`. `attach` joins one by name or id; afterwards `exec` with that `session` runs in its shared working directory, environment and shell state. `activity` returns what the other participants ran and printed there since the last call. `close` ends a session this server opened or detaches from a joined one. Only sessions of the same OS user are visible.",
            json!({
                "action": {"type": "string", "enum": ["list", "attach", "activity", "close"], "description": "list (default), attach, activity or close."},
                "session": {"type": "string", "description": "Session name or id; needed for attach, activity and close."}
            }), &[], changes(false, true)),
        tool("masks", "Mask lists", super::masks_tool::DESCRIPTION, super::masks_tool::properties(), &["action"], local_change()),
        // Not read-only: `audio` with `on` stores the viewer's „Ton“ (`screen_tool::audio`).
        tool("screen", "Screen of paired devices", screen_tool::DESCRIPTION, screen_tool::properties(), &["action"], changes(false, true)),
        tool("screen_window", "Open a native remote window viewer", super::screen_window_tool::DESCRIPTION, super::screen_window_tool::properties(), &["device", "source"], changes(true, false)),
        tool("screen_input", "Input on the screen of a paired device", screen_tool::INPUT_DESCRIPTION, screen_tool::input_properties(), &["device"], changes(true, false)),
        tool("tcp_forward", "TCP and services of paired devices", super::tcp_tool::DESCRIPTION, super::tcp_tool::properties(), &["action"], changes(true, false)),
        tool("apps", "Installed application catalog and launch", super::apps_tool::DESCRIPTION, super::apps_tool::properties(), &["action"], changes(true, false)),
        tool("rdp", "Remote Desktop to a paired device", super::rdp_tool::DESCRIPTION, super::rdp_tool::properties(), &["action"], changes(true, false)),
        tool("login", "Login screen of a paired device", super::login_tool::DESCRIPTION, super::login_tool::properties(), &["action"], changes(true, false)),
    ];
    tools.into_iter().map(upfront).collect()
}

fn upfront(tool: Tool) -> Tool {
    if !UPFRONT.contains(&tool.name.as_ref()) {
        return tool;
    }
    let mut meta = MetaObject::new();
    meta.insert("anthropic/alwaysLoad".into(), json!(true));
    tool.with_meta(meta)
}

fn transfer_tool(name: &'static str, title: &str, description: &'static str) -> Tool {
    tool(
        name,
        title,
        description,
        json!({
            "source": {"type": "string", "description": TARGET},
            "destination": {"type": "string", "description": TARGET},
            "folder_conflict": {"type":"string","enum":["merge","keep-both","fail"],"default":"merge","description":"Existing directory conflicts: merge contents, create a distinct destination name, or fail. Separate from overwrite, which controls existing files. The job result reports the actual destination alias."},
            "recursive": {"type": "boolean", "description": "Required for directories."},
            "overwrite": {"type": "boolean", "description": "Replace existing destination files (default false)."},
            "file_conflict": {"type":"string","enum":["update"],"description":"Same-named files: `update` inserts missing files, replaces older files by newer ones (by modification time, within the coarser time resolution of the two sides), skips equal files, keeps a newer destination file and keeps both when the time is equal and the size differs. Merges into an existing destination folder; not combinable with overwrite or another folder_conflict; move only between local paths. The job result reports the decisions as `update`."},
            "wait_seconds": {"type": "integer", "minimum": 0, "description": WAIT}
        }),
        &["source", "destination"],
        changes(true, false),
    )
}

#[derive(Clone, Copy)]
struct Effects {
    read_only: bool,
    destructive: bool,
    idempotent: bool,
    open_world: bool,
}

/// Reads only; `open_world` when it reaches remote systems.
const fn read_only(open_world: bool) -> Effects {
    Effects {
        read_only: true,
        destructive: false,
        idempotent: true,
        open_world,
    }
}

/// Changes only this device's own settings (the mask lists); repeating an
/// addition adds it again.
const fn local_change() -> Effects {
    Effects {
        read_only: false,
        destructive: false,
        idempotent: false,
        open_world: false,
    }
}

/// Changes files or runs commands, possibly on remote systems.
const fn changes(destructive: bool, idempotent: bool) -> Effects {
    Effects {
        read_only: false,
        destructive,
        idempotent,
        open_world: true,
    }
}

fn tool(
    name: &'static str,
    title: &str,
    description: &'static str,
    properties: Value,
    required: &[&str],
    effects: Effects,
) -> Tool {
    let mut schema = JsonObject::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), properties);
    if !required.is_empty() {
        schema.insert("required".into(), json!(required));
    }
    schema.insert("additionalProperties".into(), json!(false));
    let annotations = ToolAnnotations::with_title(title)
        .read_only(effects.read_only)
        .destructive(effects.destructive)
        .idempotent(effects.idempotent)
        .open_world(effects.open_world);
    Tool::new(name, description, schema)
        .with_title(title)
        .annotate(annotations)
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "abgleich_task_catalog_tests.rs"]
mod abgleich_task_tests;
