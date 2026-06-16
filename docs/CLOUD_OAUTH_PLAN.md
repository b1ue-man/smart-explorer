# Cloud integrations via OAuth (#19) — implementation plan

Status: **designed, ready to implement.** One external prerequisite is needed
from the maintainer before the live flow can work (see "What I need from you").

Goal: browse and **sync** cloud storage (starting with **Google Drive**) through
the same `vfs::Backend` trait the remote layer already uses — so Drive shows up
in the sidebar and the in-app picker exactly like SFTP or a network share, and a
sync setup can target it.

---

## Why this needs a step from you

Google (and Dropbox/OneDrive) require an **OAuth client registered to a project
you own**. A desktop app like this cannot ship a usable shared secret:

- Embedding a client secret in an open-source binary is not secret, and Google
  will reject/limit it.
- Using **Drive** scopes triggers Google's app-verification (consent screen,
  possibly a security review) which only the **project owner** can complete.

So the correct, standard model for an open-source desktop app is: **each user
(or you, the publisher) supplies their own OAuth client ID.** The app stores it
and runs the flow with it. This is how rclone, etc. work.

### What I need from you (one-time)
1. In <https://console.cloud.google.com> → create a project.
2. **APIs & Services → Library →** enable **Google Drive API**.
3. **OAuth consent screen →** External, add your Google account as a test user
   (test mode is fine; no verification needed for your own use).
4. **Credentials → Create credentials → OAuth client ID → Desktop app.**
5. Send me (or paste into the app's new "Cloud" settings) the **Client ID**
   (and Client secret if Google issues one for the desktop type — it's not a
   real secret for installed apps, but Google's token endpoint still wants it).

With that, the flow below works end-to-end. Until then I can build and ship all
the code (it stays dormant — no client ID configured = feature simply not shown),
but I can't run the live auth here in the headless build env, so it will want a
real Windows smoke-test.

---

## OAuth flow (PKCE loopback — no inbound ports, no embedded secret reliance)

Module `oauth.rs` (generic OAuth2, provider-agnostic):

1. Generate PKCE `code_verifier` (43–128 char URL-safe) + `code_challenge`
   (`BASE64URL(SHA256(verifier))`). (`base64` + `sha2`/ring already in tree.)
2. Start a `std::net::TcpListener` on `127.0.0.1:0` (ephemeral port) — the
   redirect URI is `http://127.0.0.1:<port>`.
3. Open the system browser (`ShellExecuteW`/`open`) to the provider auth URL with
   `client_id, redirect_uri, scope, code_challenge, S256, state`.
4. Block (with timeout) accepting one HTTP GET on the listener, parse `?code=…`
   + `state`, reply with a tiny "you can close this tab" page.
5. POST to the token endpoint (`ureq`, ring TLS) with `code, code_verifier,
   client_id, redirect_uri` → `{access_token, refresh_token, expires_in}`.
6. Store the **refresh token** in the OS keyring (`creds`), the access token +
   expiry in memory; refresh on demand.

Unit-testable without a live server: PKCE generation, auth-URL building, and the
redirect-request parsing. Token exchange/refresh need the live provider.

Config: client ID/secret + which provider in
`%APPDATA%/smart_explorer/cloud/<provider>.cfg`; refresh token in keyring under
a `cloud:<provider>:<account>` key.

---

## Google Drive backend — `gdrive.rs : impl vfs::Backend`

Drive is **ID-addressed, not path-addressed**, so the backend keeps a small
`path → fileId` cache and resolves lazily from the root.

| Backend method | Drive v3 REST (JSON; add `serde` + `serde_json`, pure Rust) |
|---|---|
| `list_dir(path)` | resolve dir→id, `files.list?q='<id>' in parents and trashed=false`, page via `nextPageToken` |
| `stat(path)` | `files.get?fileId=…&fields=id,name,mimeType,size,modifiedTime` |
| `open_read(path)` | `files.get?alt=media` (folders are `application/vnd.google-apps.folder`) |
| `open_write(path)` | resumable/multipart upload (`uploads` endpoint); create vs update by existing id |
| `mkdir_all` | create folders with the folder mime type, parent = resolved id |
| `rename` | `files.update` (name and/or `addParents`/`removeParents`) |
| `remove_file`/`remove_dir` | `files.update` trashed=true (safer) or `files.delete` |

All requests carry `Authorization: Bearer <access_token>`; a 401 triggers one
refresh + retry. `parallelism()` returns a small number (Drive rate limits).

This reuses the existing bisync engine unchanged — once Drive is a `Backend`, the
two-way sync, conflict handling, reversible backups, and the daemon all apply.

---

## Wiring (mirrors the existing remote layer; isolation first)

- `creds::Protocol::GDrive` (new arm; `is_url()`-style remote).
- A "☁ Cloud verbinden" entry in the **Verbindung** menu → runs the OAuth flow,
  then connects a `GDriveBackend` as a normal `RemoteState` (sidebar + picker
  pick it up for free).
- Sync endpoint encoding: `gdrive://<account>/<path>`; `connect::resolve_endpoint`
  gains a `gdrive` arm that rebuilds the backend from the stored refresh token —
  so cloud sync jobs run in the GUI and the background daemon, same as SFTP.
- Entirely opt-in: with no client ID configured the menu entry is hidden, so
  existing users are unaffected.

## Delivery slices (each its own release)
1. `oauth.rs` + config/keyring storage + unit tests (dormant). **← start here**
2. `gdrive.rs` browse + read (`list_dir`/`stat`/`open_read`) → navigate + copy FROM Drive.
3. Drive writes (`open_write`/`mkdir`/`rename`/`remove`) → two-way sync TO Drive.
4. Generalize to Dropbox/OneDrive (same `oauth.rs`, new `Backend` impls).

## Verification limits
The live OAuth + Drive calls can't be exercised in this headless build env. Each
slice will compile for host + `x86_64-pc-windows-gnu` and unit-test the pure
parts; the networked parts need a real Windows run with your client ID.
