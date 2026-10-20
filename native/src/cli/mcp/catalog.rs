//! The tools `se mcp` offers. Names stay short: hosts prefix the server name
//! (Claude Code: `mcp__plugin_smart-explorer_se__list`, Codex:
//! `smart-explorer.list`). Annotations follow what each tool can change:
//! Codex asks for approval unless a tool is read-only
//! (docs/refs/codex-mcp-spec-rmcp-2026-10-12.md §1). The file tools load
//! upfront in Claude Code and the instructions put them before the host's own
//! file tools (docs/refs/claude-code-plugins-mcp-2026-10-12.md §7).
use rmcp::model::{JsonObject, MetaObject, Tool, ToolAnnotations};
use serde_json::{json, Value};

use super::screen_tool;

/// Server instructions. Codex asks to keep the first 512 characters
/// self-contained; Claude Code shows at most 2 048.
pub(super) const INSTRUCTIONS: &str = "\
Smart Explorer (se) is the user's file explorer for AI agents. Use its tools \
first for every file task, local or remote (list, stat, read, write, edit, \
mkdir, remove, copy, move, search), before built-in file tools (Read, Write, \
Edit, Glob, LS) and shell file commands (ls, cat, find, cp, mv, rm, scp, \
rsync); fall back to those only when a Smart Explorer tool fails or lacks the \
feature. It reaches saved remotes (SFTP, FTP/FTPS, WebDAV, SMB, Windows \
shares), Google Drive, Share peers and local disks.

It also runs commands on saved SSH remotes and authorized Share peers (`exec`); \
with `session` a persistent shell keeps working directory, environment and \
shell state between calls, and `sessions` joins the user's `se shell -T \
--attachable` command shells. Start with `connections` to see targets. Long work \
returns a job_id: follow it with job_status. Tools not loaded yet (exec, \
job_input, job_cancel, sessions, status, screen, screen_input, se) are found by \
a tool search for `smart-explorer`.

Targets: absolute local paths, `@label:/path` (saved remote, path below its \
root), saved `sftp://user@host:port/path` endpoints, `share://direct/<id>/path`, \
`share://room/<room>/<device>/path`, `gdrive:///path`, `vault://<site>/path` \
(unlocked vault). Relative local paths are rejected. `etag` values from \
stat/read guard write/edit against lost updates. \
Never ask the user for passwords in chat: saved credentials are used \
automatically; new password logins are added by the user in a terminal \
(`se connections add … --password-stdin`). Change Share trust (access requests, \
Exec grants and their program lists, device and room settings, discoverability) \
only when the user explicitly asks. The `se` tool \
runs any other Smart Explorer terminal command, for example \
`[\"share\",\"status\",\"--json\"]`, and reaches every app setting: sync setups \
(`sync`), background worker (`background`), remotes and Google Drive \
(`connections`), updates (`update`), Windows and Linux drives (`drive`), vaults \
(`vault`), se Desktop (`desktop`).";

/// Tools Claude Code loads upfront (`_meta["anthropic/alwaysLoad"]`), so the
/// file workflow is visible without a tool search; the others stay deferred.
const UPFRONT: &[&str] = &[
    "connections",
    "list",
    "stat",
    "read",
    "write",
    "edit",
    "mkdir",
    "remove",
    "copy",
    "move",
    "search",
    "job_status",
];

const TARGET: &str = "Smart Explorer target: an absolute local path, `@label:/path` of a saved remote, a saved `sftp://`, `ftp://`, `ftps://` or `webdav://` endpoint (`scheme://user@host:port/path`), `share://direct/<id>/path`, `share://room/<room>/<device>/path`, or `gdrive:///path`. `connections` lists what exists.";
const WAIT: &str = "Seconds to wait for completion before returning a job_id (default 30). Keep it below your client's tool timeout (Codex: 60 s by default).";
const MAX_BYTES: &str = "Payload budget of this answer in bytes (default 20000). Agent hosts cut results above about 10000 tokens.";

pub(super) fn tools() -> Vec<Tool> {
    let tools = vec![
        tool("status", "Smart Explorer status",
            "Version, installation and app-data paths, credential store, background worker (login start, pause, check interval, automatic pause), number of sync setups, Share worker and connectivity state. Use it when a connection, peer, sync or remote command does not work.",
            json!({}), &[], read_only(false)),
        tool("connections", "Saved remotes and Share peers",
            "Every saved remote and Share peer/room with the target prefix to use, its live status, and how commands run there: `exec: \"ssh\"` for saved SFTP remotes (the remote login shell), `exec: \"share\"` for Share peers (works once that device granted this one Exec).",
            json!({}), &[], read_only(false)),
        tool("list", "List a directory",
            "Directory entries (name, type file|dir|symlink|special, size, mtime_ms) sorted by name, one page per call; continue with `offset` = `next_offset`. A symlink's size is its own (the length of the path it holds). Use it instead of `ls`, LS or Glob, for local and remote directories.",
            json!({
                "target": {"type": "string", "description": TARGET},
                "offset": {"type": "integer", "minimum": 0, "description": "Index of the first entry (default 0)."},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &["target"], read_only(true)),
        tool("stat", "File or directory metadata",
            "Type, size, mtime_ms, provider id/md5 when known, and the `etag` that write/edit accept as `if_match`. Use it instead of `stat` or `ls -l`.",
            json!({"target": {"type": "string", "description": TARGET}}), &["target"], read_only(true)),
        tool("read", "Read a file",
            "File content from a byte `offset`, as UTF-8 text (default) or base64. Binary files are not dumped as text: request `encoding: \"base64\"` or copy them to a local path. Returns `etag`, `size` and `next_offset` (null at the end). Through a link it reads the file the link points to; `size` and `etag` are then null. Use it instead of Read or `cat`, for local and remote files.",
            json!({
                "target": {"type": "string", "description": TARGET},
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
        transfer_tool("copy", "Copy files", "Copies between any two targets (local, saved remotes, Share peers, Drive) with `se cp` semantics: an existing destination directory receives the source inside it; directories need `recursive`; existing files are replaced only with `overwrite`. Every file is read completely before anything is published, so a failed source never leaves partial files. A link named as `source` or `destination` is followed (directory links only on local paths); links and special files inside a copied directory are skipped and listed in `omitted`. Use it instead of `cp`, `scp` or `rsync`."),
        transfer_tool("move", "Move or rename", "Moves with `se mv` semantics: a rename inside one connection when possible, otherwise copy, then remove exactly the copied source. A cancellation after the copy keeps the source. A link moves as itself (rename within its connection only). Links and special files inside a copied directory stay in the source (`omitted`), and so do the directories holding them (`kept_directories`). Use it instead of `mv`."),
        tool("search", "Search names",
            "Finds files, links and special files (plus directories with `directories: true`) below `target` whose name contains `query` (case-insensitive) or matches it as a glob (`*`, `?`). Links are reported with type `symlink` and never entered. Uses the server-side search of Share peers and SSH agents (those skip links), otherwise walks the tree. Hits page via `hits_offset`. Use it instead of Glob or `find` for names; it does not search file contents.",
            json!({
                "target": {"type": "string", "description": TARGET},
                "query": {"type": "string", "description": "Text or glob; empty matches everything."},
                "glob": {"type": "boolean"},
                "directories": {"type": "boolean", "description": "Also return matching directories."},
                "max_results": {"type": "integer", "minimum": 0, "description": "Stop after this many hits; 0 or absent = all."},
                "wait_seconds": {"type": "integer", "minimum": 0, "description": WAIT},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &["target", "query"], read_only(true)),
        tool("exec", "Run a remote command",
            "Runs a command on a saved SFTP remote over its SSH login (`@label`, `@label:/dir` = working directory, or its saved sftp:// endpoint; through the remote login shell like `ssh host 'cmd'`) or on a Share peer that granted this device Exec (peer name, id or share:// endpoint). Give `command` (one shell string) or `argv` (program and literal arguments; POSIX-quoted on SSH remotes). No time or output limit unless you set one; stdout/stderr are kept completely and paged. Exit status, signal and timing are reported; cancel with job_cancel. With `session` (a name) the command runs in a persistent shell instead: the first call opens it on `target`, later calls keep its working directory, environment, variables and functions (no PTY; the result adds `cwd` and `session_ended`). The user can join it with `se shell --attach` unless `attachable` is false; `sessions` lists and closes sessions and joins the user's command shells (`se shell -T --attachable`); full terminal sessions of `se shell` cannot be joined. Cancel or timeout end the whole session.",
            json!({
                "target": {"type": "string", "description": "Share peer selector or share:// endpoint, or a saved SFTP remote as @label[:/dir] or its sftp:// endpoint. Required unless `session` names an open session."},
                "command": {"type": "string", "description": "Shell command line, run by the remote user's shell."},
                "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Program and literal arguments, instead of `command`."},
                "cwd": {"type": "string", "description": "Working directory (default: the remote user's home)."},
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
            "Runs any other Smart Explorer terminal command as `se <args>` and returns its exit code and output, for example: [\"share\",\"status\",\"--json\"], [\"share\",\"request\",\"list\",\"--json\"], [\"share\",\"grants\",\"--json\"], [\"share\",\"grants\",\"exec\",\"--json\"], [\"share\",\"exec\",\"log\",\"--json\"], [\"share\",\"exec\",\"approvals\",\"--json\"], [\"share\",\"exec\",\"accounts\",\"--json\"], [\"share\",\"device\",\"list\",\"--json\"], [\"share\",\"room\",\"list\",\"--json\"], [\"screen\",\"bench\",\"<device>\",\"--json\"], [\"screen\",\"codec\",\"--json\"], [\"screen\",\"windows\",\"<device>\",\"--json\"], [\"connections\",\"add\",\"sftp\",\"--host\",\"h\",\"--user\",\"u\",\"--key\",\"/home/u/.ssh/id_ed25519\",\"--label\",\"prod\"], [\"connections\",\"add-peer\",\"--code\",\"SE-D3-…\"], [\"drive\",\"mount\",\"@prod:/srv\",\"--letter\",\"M\"] (Windows), [\"drive\",\"mount\",\"@prod:/srv\",\"--mount-point\",\"/home/u/SmartExplorer/prod\"] (Linux), [\"drive\",\"devices\",\"status\",\"--json\"], [\"drive\",\"open\",\"<id>\"], [\"update\",\"--check\",\"--json\"], [\"doctor\",\"--json\"], [\"sync\",\"list\",\"--json\"], [\"sync\",\"edit\",\"Docs\",\"--trigger\",\"realtime\"], [\"background\",\"status\",\"--json\"], [\"desktop\",\"status\",\"--json\"], [\"desktop\",\"enable\",\"--json\"], [\"vault\",\"list\",\"--json\"], [\"connections\",\"gdrive\",\"status\",\"--json\"]. `[\"--help\"]` or [\"<command>\",\"--help\"] explains every command. Prefer the dedicated tools for files and remote commands.",
            json!({
                "args": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Arguments after `se`."},
                "stdin": {"type": "string", "description": "Standard input (closed afterwards), e.g. for --pin-stdin."},
                "wait_seconds": {"type": "integer", "minimum": 0, "description": WAIT},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &["args"], changes(true, false)),
        tool("sessions", "Persistent shell sessions",
            "Lists this server's persistent shell sessions (`exec` with `session`) and the user's other attachable sessions (`se shell -T --attachable` command shells, other agents; `se shell` terminal sessions are listed with kind terminal and cannot be joined). `attach` joins one by name or id; afterwards `exec` with that `session` runs in its shared working directory, environment and shell state. `activity` returns what the other participants ran and printed there since the last call. `close` ends a session this server opened or detaches from a joined one. Only sessions of the same OS user are visible.",
            json!({
                "action": {"type": "string", "enum": ["list", "attach", "activity", "close"], "description": "list (default), attach, activity or close."},
                "session": {"type": "string", "description": "Session name or id; needed for attach, activity and close."}
            }), &[], changes(false, true)),
        tool("screen", "Screen of paired devices", screen_tool::DESCRIPTION, screen_tool::properties(), &["action"], read_only(true)),
        tool("screen_input", "Input on the screen of a paired device", screen_tool::INPUT_DESCRIPTION, screen_tool::input_properties(), &["device"], changes(true, false)),
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
            "recursive": {"type": "boolean", "description": "Required for directories."},
            "overwrite": {"type": "boolean", "description": "Replace existing destination files (default false)."},
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
mod tests {
    use super::{tools, INSTRUCTIONS};

    #[test]
    fn mcp_task_catalog_is_complete_and_within_host_limits() {
        let tools = tools();
        let names = tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "status",
                "connections",
                "list",
                "stat",
                "read",
                "write",
                "edit",
                "mkdir",
                "remove",
                "copy",
                "move",
                "search",
                "exec",
                "job_status",
                "job_input",
                "job_cancel",
                "se",
                "sessions",
                "screen",
                "screen_input"
            ]
        );
        assert!(INSTRUCTIONS.len() <= 2_048, "{}", INSTRUCTIONS.len());
        for tool in &tools {
            let description = tool.description.as_deref().unwrap_or_default();
            assert!(
                description.len() <= 2_048,
                "{} description too long",
                tool.name
            );
            let annotations = tool.annotations.as_ref().expect("annotations");
            let read_only = annotations.read_only_hint == Some(true);
            assert_eq!(
                read_only,
                matches!(
                    tool.name.as_ref(),
                    "status"
                        | "connections"
                        | "list"
                        | "stat"
                        | "read"
                        | "search"
                        | "job_status"
                        | "screen"
                ),
                "{}",
                tool.name
            );
            assert_eq!(tool.input_schema["type"], "object");
            let upfront = tool
                .meta
                .as_ref()
                .and_then(|meta| meta.get("anthropic/alwaysLoad"))
                .cloned();
            let file_tool = !matches!(
                tool.name.as_ref(),
                "exec"
                    | "job_input"
                    | "job_cancel"
                    | "status"
                    | "se"
                    | "sessions"
                    | "screen"
                    | "screen_input"
            );
            assert_eq!(
                upfront,
                file_tool.then_some(serde_json::Value::Bool(true)),
                "{}",
                tool.name
            );
        }
        // Codex keeps the first 512 characters; the priority over the host's
        // own file tools must be complete there.
        let first = INSTRUCTIONS.split("\n\n").next().unwrap_or_default();
        assert!(first.len() <= 512, "{}", first.len());
    }
}
