//! Exact v0.5.184 file fixtures; only temporary paths, no HOME or host CLI.
use super::*;

pub(crate) const LEGACY_SKILL: &str = r###"---
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
`job_status`, `job_input`, `job_cancel`, `sessions`, `status`, `se`) uses the
same saved connections, credentials and background worker as the user's Smart
Explorer app and `se` terminal command.

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
6. Several commands that build on each other (cd, export, activated
   environments): give `exec` a `session` name. The first call opens a
   persistent shell on `target`; later calls with the same `session` keep its
   working directory, environment and shell state and need no `target`. The
   user can watch or join it with `se shell --attach <name>`; `sessions`
   (`activity`) shows what others ran there, `close` ends it. `sessions` also
   lists the user's own shells with their kind: you can join a command shell
   started with `se shell -T --attachable`. Cancel or a timeout end the whole
   session.

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
- Join a user's command shell (`sessions` with `attach`, listed when they
  started `se shell -T --attachable`) only when the user asks for it: it shares
  their shell, including what they changed or elevated there. A plain
  `se shell` in a terminal is a full terminal session (kind terminal); agents
  cannot join it.
- When something fails, call `status`; it shows the background worker, Share
  connectivity and credential store state.
- Any other Smart Explorer function is reachable through the `se` tool, e.g.
  `["share","status","--json"]`, `["drive","list"]`, `["update","--check","--json"]`;
  `["<command>","--help"]` explains each command.
"###;

fn seed(claude: bool) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let marker = json!({"version":"0.5.184","executable":root.path().join("se")});
    let description="Smart Explorer for AI agents, used before the built-in file tools: files on local disks, saved remotes (SFTP, FTP/FTPS, WebDAV, SMB), Share peers and Google Drive, and remote commands over SSH or Smart Explorer Share.";
    let files = if claude {
        vec![
        (".claude-plugin/marketplace.json",serde_json::to_vec_pretty(&json!({"name":MARKETPLACE,"owner":{"name":"Smart Explorer"},"plugins":[{"name":PLUGIN,"source":"./smart-explorer","description":description}]})).unwrap()),
        ("smart-explorer/.claude-plugin/plugin.json",serde_json::to_vec_pretty(&json!({"name":PLUGIN,"displayName":"Smart Explorer","version":"0.5.184","description":description,"author":{"name":"Smart Explorer"},"homepage":"https://github.com/b1ue-man/smart-explorer","license":"MIT"})).unwrap()),
        ("smart-explorer/.mcp.json",serde_json::to_vec_pretty(&json!({"mcpServers":{"se":{"command":marker["executable"],"args":["mcp"]}}})).unwrap()),
        ("smart-explorer/skills/smart-explorer/SKILL.md",LEGACY_SKILL.as_bytes().to_vec()),
    ]
    } else {
        vec![("SKILL.md", LEGACY_SKILL.as_bytes().to_vec())]
    };
    for (name, bytes) in files {
        let path = root.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    std::fs::write(
        root.path().join(MARKER),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();
    root
}

#[test]
fn batches_p13_legacy_release_profiles_uninstall_directly_and_keep_unowned_files() {
    assert_eq!(managed::hash(LEGACY_SKILL.as_bytes()), LEGACY_SKILL_SHA256);
    for claude in [false, true] {
        let root = seed(claude);
        std::fs::write(root.path().join("user-notes.txt"), b"keep user's own notes").unwrap();
        verify_owned(root.path()).unwrap();
        assert!(remove_owned_files(root.path()).unwrap());
        assert_eq!(
            std::fs::read(root.path().join("user-notes.txt")).unwrap(),
            b"keep user's own notes"
        );
        assert!(!root.path().join(MARKER).exists());
        assert!(!root
            .path()
            .join(if claude {
                "smart-explorer/.mcp.json"
            } else {
                "SKILL.md"
            })
            .exists());
        assert!(
            generated_files_absent(root.path()).unwrap(),
            "unknown notes and empty generated directories do not block a later uninstall"
        );
    }
}

#[test]
fn batches_p13_legacy_uninstall_preserves_modified_skill_and_shared_mcp_fields() {
    for claude in [false, true] {
        let root = seed(claude);
        let skill = root.path().join(if claude {
            "smart-explorer/skills/smart-explorer/SKILL.md"
        } else {
            "SKILL.md"
        });
        std::fs::write(&skill, b"user's edited skill").unwrap();
        assert!(remove_owned_files(root.path())
            .unwrap_err()
            .contains("legacy owned content changed"));
        assert_eq!(std::fs::read(skill).unwrap(), b"user's edited skill");
        assert!(root.path().join(MARKER).exists());
    }
    for field in ["env", "other_server", "command", "args"] {
        let root = seed(true);
        let path = root.path().join("smart-explorer/.mcp.json");
        let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        match field {
            "env" => value["mcpServers"]["se"]["env"] = json!({"KEEP":"user"}),
            "other_server" => value["mcpServers"]["user"] = json!({"command":"user-tool"}),
            "command" => value["mcpServers"]["se"]["command"] = json!("user-tool"),
            _ => value["mcpServers"]["se"]["args"] = json!(["mcp", "--user"]),
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(remove_owned_files(root.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(root.path().join(MARKER).exists());
    }
}

#[test]
fn batches_p13_legacy_marker_cannot_add_deletion_paths() {
    let root = seed(false);
    let user = root.path().join("keep.txt");
    std::fs::write(&user, b"user").unwrap();
    let mut marker = read_marker(root.path()).unwrap();
    marker["fingerprints"] = json!({"../keep.txt":managed::hash(b"user")});
    std::fs::write(
        root.path().join(MARKER),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();
    assert!(remove_owned_files(root.path()).is_err());
    assert_eq!(std::fs::read(user).unwrap(), b"user");
    assert!(root.path().join("SKILL.md").exists());
}
