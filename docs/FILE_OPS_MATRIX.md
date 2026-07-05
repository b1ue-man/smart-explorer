# File operations - backend x API matrix

Maps every file operation to the concrete API call on each backend, with
status. Backends: **Local** (`vfs::LocalBackend`, std::fs), **SFTP**
(`sftp.rs`, russh-sftp -> SSH-FXP packets), **FTP/FTPS** (`ftp.rs`, suppaftp ->
RFC 959 commands), **WebDAV** (`webdav.rs`, RFC 4918 methods), **Drive**
(`gdrive.rs`, Drive v3 REST), and **Peer via Share-Server**
(`share::PeerBackend` over persistent DirectContact/RoomDevice profiles,
Noise-encrypted direct TCP; `se-share-server` is rendezvous only).

Status: ✅ implemented & wired · ⚠️ partial/limitation · ❌ not supported.

## A. `vfs::Backend` primitives -> documented API

| Backend method | Local (std::fs) | SFTP (SSH-FXP) | FTP (RFC 959) | WebDAV (RFC 4918) | Google Drive v3 | Peer via Share-Server |
|---|---|---|---|---|---|---|
| `list_dir` | `read_dir` ✅ | `OPENDIR`+`READDIR` ✅ | `LIST`/`MLSD` ✅ | `PROPFIND` Depth:1 ✅ | `files.list?q='<id>' in parents and trashed=false` (paged) ✅ | `FsRequest::ListDir` -> exported root / saved connection ✅ |
| `stat` | `symlink_metadata` ✅ | `LSTAT`/`STAT` ✅ | `SIZE`+`MDTM` ✅ | `PROPFIND` Depth:0 ✅ | `files.get?fields=...` ✅ | `FsRequest::Stat` ✅ |
| `exists` | `metadata.is_ok` ✅ | `STAT` ✅ | `SIZE`/list ✅ | `PROPFIND` ✅ | `files.get` / `find_child` ✅ | `stat` over peer ✅ |
| `open_read` | `File::open` ✅ | `OPEN`(READ)+`READ` ✅ | `RETR` ✅ | `GET` ✅ | `files.get?alt=media` ✅ | `FsRequest::Read` + encrypted data frames ✅ |
| `open_write` | `File::create` ✅ | `OPEN`(WRITE\|CREAT)+`WRITE` ✅ | `STOR` ✅ | `PUT` ✅ | `files.create`/`files.update` multipart upload (flush) ✅ | `FsRequest::Write` + encrypted data frames + `WriteDone` ✅ |
| `mkdir_all` | `create_dir_all` ✅ | `MKDIR` ✅ | `MKD` ✅ | `MKCOL` ✅ | `files.create` mimeType=folder ✅ | `FsRequest::MkdirAll` ✅ |
| `rename` | `fs::rename` ✅ | `RENAME` ✅ | `RNFR`+`RNTO` ✅ | `MOVE` ✅ | `files.update` (name; addParents/removeParents) ✅ | `FsRequest::Rename` within one exported mount ✅ |
| `remove_file` | `fs::remove_file` ✅ | `REMOVE` ✅ | `DELE` ✅ | `DELETE` ✅ | `files.update trashed=true` (to Drive trash) ✅ | `FsRequest::RemoveFile` ✅ |
| `remove_dir` | `fs::remove_dir` primitive; UI uses trash ✅ | `RMDIR`* ✅ | `RMD`* ✅ | `DELETE` ✅ | `files.update trashed=true` ✅ | `FsRequest::RemoveDir` recurses server-side, then removes ✅ |
| `copy_file` (same backend) | `fs::copy` ✅ | default read->write ✅ | default ✅ | default ✅ | default read->write ✅ (Drive `files.copy` not used) | default peer read->write ✅ |

\* SFTP/FTP `RMDIR`/`RMD` require an empty directory at the protocol level; UI
delete paths must walk children first when deleting non-empty folders.

Refs: [Drive v3 files](https://developers.google.com/drive/api/v3/reference),
[files.create (upload)](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/create),
WebDAV [RFC 4918](https://datatracker.ietf.org/doc/html/rfc4918),
SFTP [draft-ietf-secsh-filexfer], FTP [RFC 959].

## B. UI action -> backend method -> status per backend type

| UI action | Routed to | Local | Remote (SFTP/FTP/WebDAV/Drive) | Peer via Share-Server |
|---|---|---|---|---|
| Navigate into folder | scanner / `rscan` -> `list_dir` | ✅ | ✅ | ✅ |
| Open file (double-click/Enter) | `open_file` -> temp download + launch | ✅ | ✅ temp copy + save-back | ✅ temp copy + save-back |
| New folder | `create_new_folder` -> `mkdir_all` | ✅ | ✅ | ✅ |
| Delete (Entf) | `trash_selected` -> trash / recursive `remove_*` | ✅ | ✅ (Drive -> trash; SFTP/FTP dirs walked by app) | ✅ recursive peer delete |
| Rename (F2) | `confirm_rename` -> `rename` | ✅ | ✅ | ✅ within same peer mount |
| Right-click menu | shell menu / egui menu | ✅ shell | ✅ egui | ✅ egui |
| Copy -> paste into folder | clipboard / upload | ✅ | ✅ paste into remote = `open_write` | ✅ paste into peer = `open_write` |
| Copy files/folders -> Explorer | CF_HDROP / temp+CF_HDROP | ✅ | ✅ remote selections -> temp -> CF_HDROP | ✅ peer selections -> temp -> CF_HDROP |
| Mirror / two-way sync | `sync`/`bisync` over `Backend` | ✅ | ✅ | ⚠️ works while peer is reachable; peer TCP channel is reopened on demand |
| Drag rows between tabs/panes | internal drag -> copy/upload/download/cross-copy | ✅ local<->local | ✅ local<->remote, remote<->local, remote<->remote | ✅ via peer `Backend` |
| Drag out to Explorer (OLE) | `dragout.rs` CF_HDROP | ✅ local | ✅ remote -> temp -> OLE | ✅ peer -> temp -> OLE |
| Drop OS files into folder | `handle_os_drop` | ✅ copy | ✅ upload into remote | ✅ upload into peer |

## C. Terminal `se` command -> backend method -> status

The terminal companion uses the same app-data directory, saved connections,
keyring entries, Share profiles, and daemon worker as the GUI. Targets can be
full endpoints, local paths, or saved-connection shorthand:
`@label-or-account:/path`.

| CLI command | Routed to | Local | Remote (SFTP/FTP/WebDAV/Drive) | Peer via Share-Server |
|---|---|---|---|---|
| `se connections list` | `creds::load_connections()` + `ShareProfiles::load()` | n/a | ✅ saved remotes | ✅ saved Share contacts/rooms |
| `se connections add` | `creds::save_connection()` + keyring secret | n/a | ✅ SFTP/FTP/FTPS/WebDAV/UNC setup | n/a |
| `se connections add-peer` / `add-room` | `ShareProfiles::{add_direct_from_code, add_room_from_code}` + daemon refresh/request | n/a | n/a | ✅ one-sided setup; other peer confirms direct access as usual |
| `se ls` / `se stat` | `list_dir` / `stat` | ✅ | ✅ | ✅ |
| `se cat` / `se get` | `open_read` | ✅ | ✅ | ✅ |
| `se put` / `se mkdir` | `open_write` / `mkdir_all` | ✅ | ✅ | ✅ |
| `se cp` | read -> write, recursive with `--recursive` | ✅ | ✅ cross-backend | ✅ cross-backend |
| `se mv` | rename when possible, else copy+delete | ✅ | ✅ | ✅ |
| `se rm` | `remove_file` / recursive child walk + `remove_dir` | ✅ guarded by `--force` | ✅ guarded by `--force` | ✅ guarded by `--force` |
| `se search` | backend search, fallback traversal | ✅ | ✅ | ✅ |
| `se exec` | Share daemon IPC -> peer exec request | n/a | ❌ SFTP-agent execution out of scope | ✅ opt-in per export |

## D. Current caveats

1. **Peer identities are durable; channels are not.** Direct contacts, rooms,
   room members, trust pins, secrets, auto-connect flags, and export scopes are
   persisted. Each file operation opens a fresh authenticated peer channel after
   validating signed presence and a Noise static key.
2. **No relay/TURN.** Peer browsing requires a direct TCP path to one advertised
   candidate. The rendezvous server never relays file data.
3. **Own saved connections are exported one level deep.** A peer can browse the
   exporting device's saved SFTP/FTP/WebDAV/UNC connections when explicitly
   enabled, but peer-share connections are not persisted and therefore cannot
   recurse back into Share-Server sessions.
4. **Peer remote execution is default-deny.** `se exec share://... -- argv` works
   only for Smart Explorer Share peers whose receiving export enables
   `allow_exec`; `--shell` also requires `allow_shell_exec`.
5. **Remote clipboard and drag-out materialize eagerly.** Remote files/folders
   are downloaded to temp paths before CF_HDROP/OLE hands them to Explorer; very
   large folders therefore need enough local temp space before the drop/paste.

## E. Remote file opening

| Strategy | Mechanism | Path seen by app | Save-back | Notes |
|---|---|---|---|---|
| Temp copy + watch | `download_to_temp` -> launch -> watch mtime -> `open_write` on save | `%TEMP%\...` | ✅ on save (re-upload) | current implementation; universal, all backends/apps; client-side tracking |
| CfAPI placeholder (historical/revive-only goal) | would require `CfRegisterSyncRoot` + placeholders; `FETCH_DATA` -> `open_read`; OS notifications -> `open_write` | real local path under a sync root | not active | no current `cfprovider.rs`/`cfsync.rs`; see `docs/REMOTE_EDIT.md` and `docs/CFAPI_REVIEW.md` before reviving |
