//! The tools `se mcp` offers. Names stay short: hosts prefix the server name
//! (Claude Code: `mcp__plugin_smart-explorer_se__list`, Codex:
//! `smart-explorer.list`). Annotations follow what each tool can change:
//! Codex asks for approval unless a tool is read-only
//! (docs/refs/codex-mcp-spec-rmcp-2026-10-12.md §1).
use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use serde_json::{json, Value};

/// Server instructions. Codex asks to keep the first 512 characters
/// self-contained; Claude Code shows at most 2 048.
pub(super) const INSTRUCTIONS: &str = "\
Smart Explorer (se) gives you the user's saved remotes (SFTP, FTP/FTPS, WebDAV, \
SMB, Windows shares), Google Drive, Smart Explorer Share peers and local files, \
plus remote command execution on saved SSH remotes and authorized Share peers. \
Start with `connections` to see targets, then use list/read/write/edit/copy/move/\
remove/search/exec. Long work returns a job_id: follow it with job_status.

Targets: absolute local paths, `@label:/path` (saved remote, path below its \
root), saved `sftp://user@host:port/path` endpoints, `share://direct/<id>/path`, \
`share://room/<room>/<device>/path`, `gdrive:///path`. Relative local paths are \
rejected. `etag` values from stat/read guard write/edit against lost updates. \
Never ask the user for passwords in chat: saved credentials are used \
automatically; new password logins are added by the user in a terminal \
(`se connections add … --password-stdin`). Change Share trust (access requests, \
Exec grants, discoverability) only when the user explicitly asks. The `se` tool \
runs any other Smart Explorer terminal command, for example \
`[\"share\",\"status\",\"--json\"]`.";

const TARGET: &str = "Smart Explorer target: an absolute local path, `@label:/path` of a saved remote, a saved `sftp://`, `ftp://`, `ftps://` or `webdav://` endpoint (`scheme://user@host:port/path`), `share://direct/<id>/path`, `share://room/<room>/<device>/path`, or `gdrive:///path`. `connections` lists what exists.";
const WAIT: &str = "Seconds to wait for completion before returning a job_id (default 30). Keep it below your client's tool timeout (Codex: 60 s by default).";
const MAX_BYTES: &str = "Payload budget of this answer in bytes (default 20000). Agent hosts cut results above about 10000 tokens.";

pub(super) fn tools() -> Vec<Tool> {
    vec![
        tool("status", "Smart Explorer status",
            "Version, installation and app-data paths, credential store, background worker, Share worker and connectivity state. Use it when a connection, peer or remote command does not work.",
            json!({}), &[], read_only(false)),
        tool("connections", "Saved remotes and Share peers",
            "Every saved remote and Share peer/room with the target prefix to use, its live status, and how commands run there: `exec: \"ssh\"` for saved SFTP remotes (the remote login shell), `exec: \"share\"` for Share peers (works once that device granted this one Exec).",
            json!({}), &[], read_only(false)),
        tool("list", "List a directory",
            "Directory entries (name, type file|dir|symlink|special, size, mtime_ms) sorted by name, one page per call; continue with `offset` = `next_offset`. A symlink's size is its own (the length of the path it holds).",
            json!({
                "target": {"type": "string", "description": TARGET},
                "offset": {"type": "integer", "minimum": 0, "description": "Index of the first entry (default 0)."},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &["target"], read_only(true)),
        tool("stat", "File or directory metadata",
            "Type, size, mtime_ms, provider id/md5 when known, and the `etag` that write/edit accept as `if_match`.",
            json!({"target": {"type": "string", "description": TARGET}}), &["target"], read_only(true)),
        tool("read", "Read a file",
            "File content from a byte `offset`, as UTF-8 text (default) or base64. Binary files are not dumped as text: request `encoding: \"base64\"` or copy them to a local path. Returns `etag`, `size` and `next_offset` (null at the end). Through a link it reads the file the link points to; `size` and `etag` are then null.",
            json!({
                "target": {"type": "string", "description": TARGET},
                "offset": {"type": "integer", "minimum": 0, "description": "First byte (default 0)."},
                "length": {"type": "integer", "minimum": 1, "description": "Bytes to read (default: the answer budget)."},
                "encoding": {"type": "string", "enum": ["text", "base64"], "description": "text (default) or base64."},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &["target"], read_only(true)),
        tool("write", "Write a file",
            "Creates a file, or replaces it with `overwrite: true`; missing parent directories are created. The content is staged next to the destination and published only when complete. `if_match` (an etag from stat/read) refuses the write if the file changed meanwhile. For large files write a local file and use `copy`.",
            json!({
                "target": {"type": "string", "description": TARGET},
                "content": {"type": "string", "description": "New file content."},
                "encoding": {"type": "string", "enum": ["text", "base64"], "description": "How `content` is encoded (default text = UTF-8)."},
                "overwrite": {"type": "boolean", "description": "Replace an existing file (default false)."},
                "if_match": {"type": "string", "description": "Only write when the current file still has this etag."}
            }), &["target", "content"], changes(true, false)),
        tool("edit", "Edit a text file",
            "Applies exact text replacements to a UTF-8 file in order and saves it the same way as `write`. Each `old_text` must occur exactly once unless `replace_all` is set. Pass the etag of your last read as `if_match` to avoid overwriting someone else's change.",
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
            "Creates the directory and missing parents; an existing directory is fine.",
            json!({"target": {"type": "string", "description": TARGET}}), &["target"], changes(false, true)),
        tool("remove", "Remove a file or directory",
            "Deletes a file, or a directory with `recursive: true` (links inside are removed, never followed). Filesystem roots and a saved connection's configured root need `allow_root: true`. Like `se rm --force`; there is no undo.",
            json!({
                "target": {"type": "string", "description": TARGET},
                "recursive": {"type": "boolean", "description": "Required for directories."},
                "allow_root": {"type": "boolean", "description": "Allow removing a filesystem root or a saved connection's root."}
            }), &["target"], changes(true, true)),
        transfer_tool("copy", "Copy files", "Copies between any two targets (local, saved remotes, Share peers, Drive) with `se cp` semantics: an existing destination directory receives the source inside it; directories need `recursive`; existing files are replaced only with `overwrite`. Every file is read completely before anything is published, so a failed source never leaves partial files. A link named as `source` or `destination` is followed (directory links only on local paths); links and special files inside a copied directory are skipped and listed in `omitted`."),
        transfer_tool("move", "Move or rename", "Moves with `se mv` semantics: a rename inside one connection when possible, otherwise copy, then remove exactly the copied source. A cancellation after the copy keeps the source. A link moves as itself (rename within its connection only). Links and special files inside a copied directory stay in the source (`omitted`), and so do the directories holding them (`kept_directories`)."),
        tool("search", "Search names",
            "Finds files, links and special files (plus directories with `directories: true`) below `target` whose name contains `query` (case-insensitive) or matches it as a glob (`*`, `?`). Links are reported with type `symlink` and never entered. Uses the server-side search of Share peers and SSH agents (those skip links), otherwise walks the tree. Hits page via `hits_offset`.",
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
            "Runs a command on a saved SFTP remote over its SSH login (`@label`, `@label:/dir` = working directory, or its saved sftp:// endpoint; through the remote login shell like `ssh host 'cmd'`) or on a Share peer that granted this device Exec (peer name, id or share:// endpoint). Give `command` (one shell string) or `argv` (program and literal arguments; POSIX-quoted on SSH remotes). No time or output limit unless you set one; stdout/stderr are kept completely and paged. Exit status, signal and timing are reported; cancel with job_cancel.",
            json!({
                "target": {"type": "string", "description": "Share peer selector or share:// endpoint, or a saved SFTP remote as @label[:/dir] or its sftp:// endpoint."},
                "command": {"type": "string", "description": "Shell command line, run by the remote user's shell."},
                "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Program and literal arguments, instead of `command`."},
                "cwd": {"type": "string", "description": "Working directory (default: the remote user's home)."},
                "env": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Extra environment variables."},
                "stdin": {"type": "string", "description": "Text written to the command's standard input, followed by end-of-input unless `keep_stdin_open`."},
                "keep_stdin_open": {"type": "boolean", "description": "Leave standard input open for job_input."},
                "timeout_seconds": {"type": "integer", "minimum": 1, "description": "Stop the command after this many seconds (default: no limit)."},
                "wait_seconds": {"type": "integer", "minimum": 0, "description": WAIT},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &["target"], changes(true, false)),
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
            "Runs any other Smart Explorer terminal command as `se <args>` and returns its exit code and output, for example: [\"share\",\"status\",\"--json\"], [\"share\",\"request\",\"list\",\"--json\"], [\"share\",\"grants\",\"--json\"], [\"connections\",\"add\",\"sftp\",\"--host\",\"h\",\"--user\",\"u\",\"--key\",\"/home/u/.ssh/id_ed25519\",\"--label\",\"prod\"], [\"connections\",\"add-peer\",\"--code\",\"SE-D3-…\"], [\"drive\",\"mount\",\"@prod:/srv\",\"--letter\",\"M\"] (Windows), [\"update\",\"--check\",\"--json\"], [\"doctor\",\"--json\"]. `[\"--help\"]` or [\"<command>\",\"--help\"] explains every command. Prefer the dedicated tools for files and remote commands.",
            json!({
                "args": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "Arguments after `se`."},
                "stdin": {"type": "string", "description": "Standard input (closed afterwards), e.g. for --pin-stdin."},
                "wait_seconds": {"type": "integer", "minimum": 0, "description": WAIT},
                "max_bytes": {"type": "integer", "minimum": 1024, "description": MAX_BYTES}
            }), &["args"], changes(true, false)),
    ]
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
                "se"
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
                    "status" | "connections" | "list" | "stat" | "read" | "search" | "job_status"
                ),
                "{}",
                tool.name
            );
            assert_eq!(tool.input_schema["type"], "object");
        }
    }
}
