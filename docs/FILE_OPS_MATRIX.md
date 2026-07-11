# File operations - backend x API matrix

Maps every file operation to the concrete API call on each backend, with
status. Backends: **Local** (`vfs::LocalBackend`, std::fs), **SFTP**
(`sftp.rs`, russh-sftp -> SSH-FXP packets), **FTP/FTPS** (`ftp.rs`, suppaftp ->
RFC 959 commands), **WebDAV** (`webdav.rs`, RFC 4918 methods), **Drive**
(`gdrive/`, Drive v3 REST), and **Peer via Share-Server**
(`share::PeerBackend` over persistent DirectContact/RoomDevice profiles and
authenticated Iroh/QUIC sessions). Iroh tries a direct path first and can use
the relay bundled into `se-share-server`; either path carries end-to-end
encrypted peer frames.

Status: ✅ implemented & wired · ⚠️ partial/limitation · ❌ not supported.

## A. `vfs::Backend` primitives -> documented API

| Backend method | Local (std::fs) | SFTP (SSH-FXP) | FTP (RFC 959) | WebDAV (RFC 4918) | Google Drive v3 | Peer via Share-Server |
|---|---|---|---|---|---|---|
| `list_dir` | `read_dir` ✅ | `OPENDIR`+`READDIR` ✅ | `LIST`/`MLSD` ✅ | `PROPFIND` Depth:1 ✅ | `files.list?q='<id>' in parents and trashed=false` (paged) ✅ | `FsRequest::ListDir` -> exported root / saved connection ✅ |
| `stat` | `symlink_metadata` ✅ | `LSTAT`/`STAT` ✅ | `SIZE`+`MDTM` ✅ | `PROPFIND` Depth:0 ✅ | `files.get?fields=...` ✅ | `FsRequest::Stat` ✅ |
| `exists` | `metadata.is_ok` ✅ | `STAT` ✅ | `SIZE`/list ✅ | `PROPFIND` ✅ | `files.get` / `find_child` ✅ | `stat` over peer ✅ |
| `open_read` | `File::open` ✅ | `OPEN`(READ)+`READ` ✅ | `RETR` ✅ | `GET` ✅ | `files.get?alt=media` ✅ | `FsRequest::Read` + encrypted data frames ✅ |
| `open_write` | `File::create` ✅ | `OPEN`(WRITE\|CREAT)+`WRITE` ✅ | `STOR` ✅ | `PUT` ✅ | disk-backed spool -> `files.create`/`files.update` resumable upload with retry/status recovery; flush commits and verifies size+MD5 ✅ | `FsRequest::Write` + encrypted data frames + `WriteDone` ✅ |
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
| Mirror / two-way sync | `sync`/`bisync` over `Backend` | ✅ | ✅ | ⚠️ works while peer is reachable; each operation opens a fresh QUIC stream and reconnects the cached Iroh session when needed |
| Drag rows between tabs/panes | internal drag -> copy/upload/download/cross-copy | ✅ local<->local | ✅ local<->remote, remote<->local, remote<->remote | ✅ via peer `Backend` |
| Drag out to Explorer (OLE) | `dragout.rs` CF_HDROP | ✅ local | ✅ remote -> temp -> OLE | ✅ peer -> temp -> OLE |
| Drop OS files into folder | `handle_os_drop` | ✅ copy | ✅ upload into remote | ✅ upload into peer |

## C. Terminal `se` command -> backend method -> status

The terminal companion works without a graphical session and uses the same
app-data directory, saved connections, credential records, Share profiles, and
daemon worker as the GUI. Targets can be
full endpoints, local paths, or saved-connection shorthand:
`@label-or-account:/path`.

| CLI command | Routed to | Local | Remote (SFTP/FTP/WebDAV/Drive) | Peer via Share-Server |
|---|---|---|---|---|
| `se doctor [--json]` | bounded, non-mutating checks of app-data, credentials, profiles, server config, and daemon heartbeat | ✅ | ✅ corruption and backend errors produce exit 1 | ✅ configuration health without starting Share |
| `se connections list` | checked credential metadata + `ShareProfiles::load_checked()` | n/a | ✅ saved remotes | ✅ saved Share contacts/rooms |
| `se connections add` / `remove` | transactional metadata + platform credential store | n/a | ✅ SFTP/FTP/FTPS/WebDAV/UNC setup and removal | n/a |
| `se connections add-peer` / `add-room` / `remove-peer` / `remove-room` | revision-checked `ShareProfiles` persistence + daemon profile reload | n/a | n/a | ✅ one-sided setup/removal; other peer confirms direct access as usual |
| `se share ...` | checked Share identity/profile persistence + authenticated local daemon IPC | n/a | n/a | ✅ configure, identity, status, requests, exports, rooms, refresh, and stop |
| `se ls` / `se stat` | `list_dir` / `stat` | ✅ | ✅ | ✅ |
| `se cat` / `se get` | `open_read` | ✅ | ✅ | ✅ |
| `se put` / `se mkdir` | `open_write` / `mkdir_all` | ✅ | ✅ | ✅ |
| `se cp` | read -> write, recursive with `--recursive` | ✅ | ✅ cross-backend | ✅ cross-backend |
| `se mv` | rename when possible, else copy+delete | ✅ | ✅ | ✅ |
| `se rm` | `remove_file` / recursive child walk + `remove_dir` | ✅ `--force`; roots also require `--no-preserve-root` | ✅ `--force`; configured roots also require `--no-preserve-root` | ✅ `--force`; virtual roots also require `--no-preserve-root` |
| `se search` | backend search, fallback traversal | ✅ | ✅ | ✅ |
| `se exec` | immediate fail-closed CLI rejection | n/a | ❌ SFTP-agent execution out of scope | ❌ runtime-disabled until full process-tree containment is available on every supported OS |

## D. Current caveats

1. **Peer identities and policy are durable; transport sessions are
   process-local.** Direct contacts, rooms, room members, trust pins, relation
   secrets, auto-connect flags, and relation-level export scopes are persisted.
   A file operation opens a fresh bidirectional stream on an authenticated Iroh
   session. The receiver revalidates the identity, grant, relation proof, and
   current export policy for every stream; a profile/auth-policy change closes
   both incoming and outgoing cached sessions.
2. **Direct-first with encrypted relay fallback.** Iroh attempts advertised IP
   paths first. By default, `se-share-server` also starts an Iroh relay on the
   signaling port plus one (configurable/disableable by environment). A selected
   relay forwards encrypted QUIC transport packets and can observe routing
   metadata/ciphertext, but it does not receive relation secrets or plaintext
   filesystem frames.
3. **Own saved connections are exported one level deep.** A peer can browse the
   exporting device's saved SFTP/FTP/WebDAV/UNC connections only when the
   default-direct or room export policy enables them. Peer-share connections are
   not persisted as nested exports and therefore cannot recurse back into Share
   sessions.
4. **Peer remote execution fails closed.** The receiver validates and clamps the
   request, applies one `allow_exec` permission for both argv and shell modes,
   and enforces global/per-peer admission before blocking work. Execution then
   returns unsupported until every supported OS can contain and tear down the
   complete descendant process tree. Submitted commands are never retried after
   an ambiguous transport error.
5. **Remote clipboard and drag-out materialize eagerly.** Remote files/folders
   are downloaded to temp paths before CF_HDROP/OLE hands them to Explorer; very
   large folders therefore need enough local temp space before the drop/paste.
6. **Linux headless credentials use owner-protected files.** The DBus-free
   backend enforces a `0700` directory, `0600` single-link records, bounded
   versioned envelopes, atomic replacement, and interprocess locking. It is not
   encryption against root, the same Unix account, or offline access to an
   unencrypted disk. Windows continues to use Credential Manager.

## E. Remote file opening

| Strategy | Mechanism | Path seen by app | Save-back | Notes |
|---|---|---|---|---|
| Temp copy + watch | `download_to_temp` -> launch -> watch mtime -> `open_write` on save | `%TEMP%\...` | ✅ on save (re-upload) | current implementation; universal, all backends/apps; client-side tracking |
| CfAPI placeholder (historical/revive-only goal) | would require `CfRegisterSyncRoot` + placeholders; `FETCH_DATA` -> `open_read`; OS notifications -> `open_write` | real local path under a sync root | not active | no current `cfprovider.rs`/`cfsync.rs`; see `docs/REMOTE_EDIT.md` and `docs/CFAPI_REVIEW.md` before reviving |
