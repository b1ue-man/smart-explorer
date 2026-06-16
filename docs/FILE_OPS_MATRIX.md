# File operations — backend × API matrix (review)

Maps every file operation to the concrete documented API call on each backend,
with status. Backends: **Local** (`vfs::LocalBackend`, std::fs), **SFTP**
(`sftp.rs`, russh-sftp → SSH-FXP packets), **FTP/FTPS** (`ftp.rs`, suppaftp →
RFC 959 commands), **WebDAV** (`webdav.rs`, RFC 4918 methods), **Drive**
(`gdrive.rs`, Drive v3 REST).

Status: ✅ implemented & wired · ⚠️ partial/limitation · ❌ not supported.

## A. `vfs::Backend` primitives → documented API

| Backend method | Local (std::fs) | SFTP (SSH-FXP) | FTP (RFC 959) | WebDAV (RFC 4918) | Google Drive v3 |
|---|---|---|---|---|---|
| `list_dir` | `read_dir` ✅ | `OPENDIR`+`READDIR` ✅ | `LIST`/`MLSD` ✅ | `PROPFIND` Depth:1 ✅ | `files.list?q='<id>' in parents and trashed=false` (paged) ✅ |
| `stat` | `symlink_metadata` ✅ | `LSTAT`/`STAT` ✅ | `SIZE`+`MDTM` ✅ | `PROPFIND` Depth:0 ✅ | `files.get?fields=…` ✅ |
| `exists` | `metadata.is_ok` ✅ | `STAT` ✅ | `SIZE`/list ✅ | `PROPFIND` ✅ | `files.get` / `find_child` ✅ |
| `open_read` | `File::open` ✅ | `OPEN`(READ)+`READ` ✅ | `RETR` ✅ | `GET` ✅ | `files.get?alt=media` ✅ |
| `open_write` | `File::create` ✅ | `OPEN`(WRITE\|CREAT)+`WRITE` ✅ | `STOR` ✅ | `PUT` ✅ | `files.create`/`files.update` **multipart upload** (flush) ✅ |
| `mkdir_all` | `create_dir_all` ✅ | `MKDIR` ✅ | `MKD` ✅ | `MKCOL` ✅ | `files.create` mimeType=folder ✅ |
| `rename` | `fs::rename` ✅ | `RENAME` ✅ | `RNFR`+`RNTO` ✅ | `MOVE` ✅ | `files.update` (name; addParents/removeParents) ✅ |
| `remove_file` | `fs::remove_file` ✅ | `REMOVE` ✅ | `DELE` ✅ | `DELETE` ✅ | `files.update trashed=true` (to Drive trash) ✅ |
| `remove_dir` | `fs::remove_dir_all` ✅ | `RMDIR`* ✅ | `RMD`* ✅ | `DELETE` ✅ | `files.update trashed=true` ✅ |
| `copy_file` (same backend) | `fs::copy` ✅ | default read→write ✅ | default ✅ | default ✅ | default read→write ✅ (Drive `files.copy` not used) |

\* SFTP/FTP `RMDIR`/`RMD` require an empty dir; the app's recursive delete walks
children first via the same primitives (TODO: confirm recursive-remote-delete is
wired for non-empty dirs — see §C).

Refs: [Drive v3 files](https://developers.google.com/drive/api/v3/reference),
[files.create (upload)](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/create),
WebDAV [RFC 4918](https://datatracker.ietf.org/doc/html/rfc4918),
SFTP [draft-ietf-secsh-filexfer], FTP [RFC 959].

## B. UI action → backend method → status per backend type

| UI action | Routed to | Local | Remote (SFTP/FTP/WebDAV/Drive) |
|---|---|---|---|
| Navigate into folder | scanner / `rscan`→`list_dir` | ✅ | ✅ |
| Open file (double-click/Enter) | `open_file` → temp download + launch | ✅ | ✅ (temp copy; **save-back + CfAPI = §D**) |
| New folder | `create_new_folder` → `mkdir_all` | ✅ | ✅ (0.5.24) |
| Delete (Entf) | `trash_selected` → trash / `remove_*` | ✅ | ✅ (0.5.24; Drive→trash) |
| Rename (F2) | `confirm_rename` → `rename` | ✅ | ✅ (0.5.24) |
| Right-click menu | shell menu / **egui menu** | ✅ shell | ✅ egui (0.5.24) |
| Copy → paste into folder | clipboard / upload | ✅ | ✅ paste **into** remote = `open_write` (0.5.20) |
| Copy file → Explorer | CF_HDROP / temp+CF_HDROP | ✅ | ✅ remote→temp→CF_HDROP (0.5.21) |
| Mirror / two-way sync | `sync`/`bisync` over `Backend` | ✅ | ✅ |
| **Drag rows between tabs/panes** | internal drag → copy/upload | ✅ local↔local | ❌ **OPEN**: remote source/target not handled |
| **Drag out to Explorer (OLE)** | `dragout.rs` CF_HDROP | ✅ local | ❌ **OPEN**: needs remote→temp materialize |
| Drop OS files into folder | `handle_os_drop` | ✅ copy | ✅ upload into remote (0.5.20) |

## C. Gaps found by this review (to implement)

1. **Drag rows between tabs/panes for remotes** — when the dragged items are
   remote (or the target pane is remote), route to download/upload/cross-backend
   copy instead of a local `fs` copy.
2. **Drag out to Explorer from a remote** — materialize selected remote files to
   a temp copy, then hand the temp paths to the OLE `DoDragDrop` (or fall back to
   the existing Ctrl+C remote→Explorer path).
3. **Recursive remote delete of non-empty dirs** — verify each backend's
   `remove_dir` (SFTP `RMDIR`, FTP `RMD` need empty dirs; WebDAV `DELETE` and
   Drive trash are recursive). If not recursive, walk + delete children first.
4. **Remote file editing/save-back** — currently open = temp copy with no
   save-back. See §D.

## D. Remote file opening — two strategies, user-toggleable (goal)

| Strategy | Mechanism | Path seen by app | Save-back | Notes |
|---|---|---|---|---|
| **Temp copy + watch** | `download_to_temp` → launch → watch mtime → `open_write` on save | `%TEMP%\…` | ✅ on save (re-upload) | universal, all backends/apps; client-side tracking |
| **CfAPI placeholder** (`cfsync.rs`, Win-only) | `CfRegisterSyncRoot` + placeholders; `FETCH_DATA`→`open_read`; `ReadDirectoryChangesW`→`open_write` | real local path under a sync root | ✅ native, transparent | barrier-free (no driver); large; untestable here |

Toggle in Einstellungen → "Remote-Dateien öffnen: Temp-Kopie ⟷ CfAPI". Decision
rationale: `docs/REMOTE_EDIT.md`.
