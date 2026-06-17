# Cheap, download-free equality checks: Google Drive v3 + WebDAV/Nextcloud/ownCloud

Goal: decide whether a remote file equals a local file **without downloading the
bytes**, using server-provided checksums + metadata, so Smart Explorer stops
re-transferring identical files just because mtime differs.

Distilled from primary docs (2026-06-17). All field/property/header names are
quoted exactly. Sources at the bottom of each section.

---

## 1. Google Drive API v3

### 1.1 The metadata fields that let us compare without downloading

From the **Files resource** reference (exact "Output only" descriptions quoted):

| Field | Exact doc wording / meaning | Present on which files |
|---|---|---|
| **`md5Checksum`** | *"Output only. The MD5 checksum for the content of the file. This is only applicable to files with **binary content** in Google Drive."* | Binary uploaded blobs only |
| **`sha1Checksum`** | *"Output only. The SHA1 checksum associated with this file, if available. This field is only populated for files with content stored in Google Drive; it's **not populated for Docs Editors or shortcut files**."* | Binary blobs only |
| **`sha256Checksum`** | *"Output only. The SHA256 checksum associated with this file, if available... **not populated for Docs Editors or shortcut files**."* | Binary blobs only |
| **`size`** | *"Output only. Size in bytes of blobs and Google Workspace editor files. **Won't be populated for files that have no size, like shortcuts and folders.**"* | Blobs + editor files (NOT folders/shortcuts) |
| **`modifiedTime`** | *"The last time the file was modified by anyone (RFC 3339 date-time)."* | All; **read/write**, server-set on upload → unreliable vs local mtime |
| **`version`** | *"Output only. A monotonically increasing version number for the file. This reflects **every change made to the file on the server**, even those not visible to the user."* | All |
| **`headRevisionId`** | *"Output only. The ID of the file's head revision. This is currently **only available for files with binary content** in Google Drive."* | Binary blobs only |
| **`quotaBytesUsed`** | *"Output only. The number of storage quota bytes used by the file..."* | All (includes prior kept revisions; ≠ content size) |
| **`mimeType`** | Google Apps editor files use `application/vnd.google-apps.*` (e.g. `…document`, `…spreadsheet`) | All |

**Which files have md5/sha vs which DON'T (confirmed by the reference):**
- **HAVE** md5/sha1/sha256 + `headRevisionId`: ordinary **binary uploaded files**
  (the ones we actually sync byte-for-byte).
- **DON'T HAVE** them: **Google Docs/Sheets/Slides editor files**
  (`mimeType` = `application/vnd.google-apps.*`), **folders**
  (`application/vnd.google-apps.folder`), and **shortcuts**. Editor files have a
  `size` but no fixed content bytes (export-only via `exportLinks`), so a content
  hash is meaningless for them.
- Caveat (rclone, corroborated): *"a small fraction of files uploaded may not have
  SHA1 or SHA256 hashes especially if they were uploaded before 2018."* `md5Checksum`
  is the most universally present of the three for old blobs; SHA256 is newest
  (added Aug 2022). **Google Photos**: checksums can be wrong because Google
  re-encodes media without updating the hash (rclone `--drive-skip-checksum-gphotos`).

### 1.2 `md5Checksum` = MD5 of the file content (comparable to a local MD5)

The reference states `md5Checksum` is *"the MD5 checksum for the content of the
file."* It is the standard MD5 over the stored content bytes. rclone treats Drive's
`md5Checksum`/`sha1Checksum`/`sha256Checksum` as the file's MD5/SHA1/SHA256 and
compares them directly to locally computed hashes (its whole hash-based check
relies on this equivalence). **So: compute the local file's MD5 (or SHA1/SHA256)
and string-compare (lowercase hex) to the Drive field — equal hash ⇒ equal content.**

### 1.3 Fetching ONLY these fields cheaply — the `fields=` partial response

Drive supports **partial responses** via the `fields` system parameter (a
FieldMask). Quoted from the perf guide: *"ask the server to send only the fields
you really need and get a partial response instead... This lets your application
avoid transferring, parsing, and storing unneeded fields, so it can use resources
including network, CPU, and memory more efficiently."*

Syntax (from the fields-parameter guide):
- Comma-separated list; nested with `/`; array/object sub-selectors with `()`; `*`
  wildcard. Examples quoted: `name, mimeType`, `capabilities/canDownload`,
  `permissions(id)`.
- **Single file** (`files.get`):
  `GET .../files/FILE_ID?fields=id,name,size,md5Checksum,modifiedTime,version,headRevisionId,mimeType`
- **List** (`files.list`) — must wrap the per-item fields and include the page
  token: `fields=nextPageToken,files(id,name,size,md5Checksum,sha1Checksum,modifiedTime,mimeType)`
- **Default if `fields` omitted on `files.list`:** *"the `list` method on the
  `files` resource only returns the `kind`, `id`, `name`, and `mimeType` fields."*
  So **you MUST add `fields=` to even get `size`/`md5Checksum`.** (`about`,
  `comments`, `replies` have no defaults and require `fields`.)

**Cost/quota notes:** `fields` reduces response **bandwidth, CPU and memory**
(official perf guidance). It does **not** lower the per-method *quota unit* cost
(quota is per-request/per-method, not per-byte), so the real win from `fields` is
data-on-the-wire + parsing, and the real win for *quota* is making **fewer
requests** — i.e. batch via `files.list` with pagination (`pageSize`,
`pageToken`) instead of N×`files.get`, and use the Changes API for deltas (below).

### 1.4 Incremental delta — the Changes API + start page token

To learn *only what changed since last sync* (instead of re-listing everything):

1. **Bootstrap:** call `changes.getStartPageToken` → returns `startPageToken`
   (*"gets the starting pageToken for listing future changes"*). Persist it.
2. **Poll:** `changes.list(pageToken=<saved>)`. Entries are *"in chronological
   order (the oldest changes appear first)."*
3. **Paginate:** follow `nextPageToken` for more pages.
4. **Checkpoint:** when there is no `nextPageToken`, the response carries
   `newStartPageToken` — *"the client application should store the
   `newStartPageToken` ... for future use"* and poll again next cycle with it.

Each change record exposes `fileId`, `removed` (deleted flag), a change `time`,
and an embedded `file` resource. **Crucially, `fields` works here too**, so you
fetch hashes inline per change:
`fields=newStartPageToken,nextPageToken,changes(fileId,removed,time,file(id,name,size,md5Checksum,modifiedTime,mimeType,trashed))`

Useful list params: `spaces` (e.g. `drive`), `includeRemoved`,
`restrictToMyDrive`, `includeItemsFromAllDrives` (+ `supportsAllDrives`),
`pageSize`. `fileId` is **stable across renames/moves**, so the Changes feed gives
us rename/move detection for free.

---

## 2. WebDAV — Nextcloud / ownCloud (oc/nc namespaces)

A single **`PROPFIND`** (Depth: 1) with a `<d:prop>` body returns all metadata for
a directory listing — no GET, no download.

### 2.1 Cheap metadata properties (exact, from Nextcloud "Basic" WebDAV docs)

Namespaces (verbatim): `xmlns:d="DAV:"`, `xmlns:oc="http://owncloud.org/ns"`,
`xmlns:nc="http://nextcloud.org/ns"`.

| Property | Exact doc wording | Use |
|---|---|---|
| `d:getcontentlength` | *"The size if it is a file in bytes."* (files only) | size compare |
| `d:getlastmodified` | *"The latest modification time."* (e.g. `Wed, 20 Jul 2022 05:12:23 GMT`) | mtime (server-set; unreliable cross-side) |
| `d:getetag` | *"The file's etag."* (e.g. `"6436d084d4805"`) | **change detection** |
| `d:getcontenttype` | *"The mime type of the file."* | type |
| `d:resourcetype` | *"Specifies the nature of the resource"* — `<d:collection/>` ⇒ folder | folder vs file |
| `oc:checksums` | *"An array of checksums stored in the DB by other clients. Currently used algorithms are: MD5, SHA1, SHA256, SHA3-256, and Adler32."* | **content equality** |
| `oc:id` | *"The fileid namespaced by the instance id. Globally unique."* (e.g. `00000007oc9l3j5ur4db`) | stable id → rename/move detection |
| `oc:fileid` | unique file id within the instance (e.g. `7`) | stable id |
| `oc:permissions` | *"The permissions that the user has over the file or folder... a string containing letters"* (S R M G D N V W C K) | capability/read-only check |
| `oc:size` | works for **folders** too (cumulative bytes), unlike `getcontentlength` | dir size |
| `oc:favorite` | `0`/`1` | n/a |
| `d:quota-used-bytes` / `d:quota-available-bytes` | quota | n/a |

### 2.2 Native checksums — `oc:checksums` + the `OC-Checksum` header

- **PROPFIND property:** `{http://owncloud.org/ns}checksums`. Returned shape (an
  `oc:checksums` element wrapping `oc:checksum`), value is a **space-separated,
  `ALGO:hex` string**, e.g.:
  `SHA1:edde2f3e9e741a77c04ad6681832333f20896ce0 MD5:e5e4d3b6b43b19ca028fbaaf144ae9b6 ADLER32:ad705297`
- **HEAD/GET header:** `OC-Checksum: SHA1:f572d396fae9206628714fb2ce00f72e94f2258f`
  (format `ALGO:hex`). On **upload** the client may send `OC-Checksum` and the
  server (ownCloud) verifies it against its own computed SHA1/MD5/ADLER32 as an
  integrity check.
- **Algorithms:** ownCloud = **SHA1, MD5, ADLER32**; Nextcloud's basic-API doc
  lists **MD5, SHA1, SHA256, SHA3-256, Adler32**. **Server capabilities** advertise
  the supported set and a preferred upload type via the OCS capabilities endpoint
  (`checksums` → `supportedTypes` / `preferredUploadType`); the desktop client uses
  the announced algorithm.
- **Reliability caveat (rclone, corroborated by ownCloud bug history):**
  *"Depending on the exact version of ownCloud or Nextcloud hashes may appear on
  all objects, or only on objects which had a hash uploaded with them."* So
  `oc:checksums` can be **absent** for files uploaded by clients that didn't send
  one. Treat a present checksum as authoritative; treat absence as "fall back to
  size + etag, or upload-with-checksum to populate it."

### 2.3 ETag semantics (change detection on WebDAV)

`d:getetag` is the WebDAV change token. The Nextcloud/ownCloud sync clients store a
per-directory ETag in a local journal; on the next scan, an **unchanged ETag ⇒
unchanged subtree** (folder ETags roll up child changes), so they skip it entirely.
ETag is opaque — compare for **equality only** vs the last-seen value; never parse
it as a hash. It is the cheapest "did anything change here?" signal and pairs with
`oc:id` for rename tracking.

---

## 3. Generic WebDAV (no `oc`/`nc` namespace) — what's available and the limits

Only the **DAV: live properties** are guaranteed by RFC 4918:

- `d:getcontentlength` — size (files only).
- `d:getlastmodified` — last-modified (server-controlled; usually **not**
  client-settable, often equals upload time, lossy granularity → unreliable for
  equality vs a local mtime).
- `d:getetag` — opaque change token (compare-equal for change detection; **not** a
  content hash; format/derivation is server-specific and not portable).
- `d:resourcetype`, `d:getcontenttype`, `d:displayname`, `d:creationdate`.

**Hard limits (rclone, verbatim):** *"Plain WebDAV does not support hashes"* and
*"Plain WebDAV does not support modified times."* (ownCloud/Nextcloud/Fastmail are
the exceptions that add SHA1/MD5 + `X-OC-Mtime` for setting mtime.)

⇒ On generic WebDAV the **only** download-free equality signals are **`size` +
`etag`**. That's not a true content-equality test (etag is opaque and may change on
metadata-only edits, or stay stable across servers differently). For certainty
without a server hash you'd have to download — so for generic WebDAV: **trust
size+etag as a heuristic; offer an opt-in "verify by download+hash" only when the
heuristic is ambiguous.**

---

## 4. How rclone uses these to avoid downloads (reference behavior)

- **Drive:** *"Hash algorithms MD5, SHA1 and SHA256 are supported."* rclone pulls
  the Drive metadata hash and compares it to the local hash — no download.
- **WebDAV:** *"Plain WebDAV does not support hashes, however when used with
  Fastmail Files, ownCloud or Nextcloud rclone will support SHA1 and MD5 hashes."*
  It reads `oc:checksums` / `OC-Checksum`. `--webdav-vendor=nextcloud|owncloud`
  selects this behavior.
- **`rclone check` (default):** *"compares sizes and hashes (MD5 or SHA1)"*
  **server-side, without downloading**; `--download` is the explicit opt-in to
  fetch both sides and hash on the fly; `--size-only` *"will only compare the sizes
  not the hashes... for a quick check."*
- **`--checksum` (global):** *"check the file hash and size to determine if files
  are equal... useful when the remote doesn't support setting modified time and a
  more accurate sync is desired than just checking the file size."* This is exactly
  Smart Explorer's problem (remote mtime ≠ local mtime on identical content).

---

## 5. Concrete recommendation for Smart Explorer

**Default comparison order — cheapest correct signal first, never download to compare:**

**Google Drive (binary blobs — the files we sync):**
1. Request metadata with a partial-response field mask:
   `files(id,name,size,md5Checksum,modifiedTime,version,headRevisionId,mimeType,trashed)`
   (and `nextPageToken` for lists). For ongoing sync use the **Changes API**
   (`getStartPageToken` → `changes.list` with the same `file(...)` mask, persist
   `newStartPageToken`) so you only see deltas; `fileId` gives rename/move tracking.
2. Compare **`size`** first (cheap reject). If sizes differ ⇒ not equal.
3. If sizes match, compute the **local file's MD5** and string-compare (lowercase
   hex) to **`md5Checksum`**. Equal ⇒ **content-equal, skip transfer** even though
   `modifiedTime` differs. (Use `sha256Checksum`/`sha1Checksum` if you prefer SHA;
   fall back to `md5Checksum` for pre-2018 blobs that may lack SHA.)
4. **Do NOT** content-hash-compare Google **editor files** (`mimeType`
   `application/vnd.google-apps.*`), **folders**, or **shortcuts** — they have no
   `md5Checksum`. For editor files, track change via **`version`** (or
   `modifiedTime`/`headRevision`) and treat them as export-only / non-byte-synced.
5. Never trust `modifiedTime` for equality — it's server-set and write-capable.

**WebDAV Nextcloud/ownCloud:**
1. One `PROPFIND` (Depth 1) requesting:
   `d:getcontentlength, d:getlastmodified, d:getetag, d:resourcetype,
   oc:id, oc:permissions, oc:checksums`.
2. Compare **`d:getcontentlength`** (size) first.
3. If sizes match and `oc:checksums` is present, parse the matching `ALGO:hex`
   (prefer SHA1, else MD5), compute the **same algorithm locally**, compare ⇒
   content-equal ⇒ skip. Use `oc:id` to detect renames/moves; cache `d:getetag`
   per path/dir to short-circuit unchanged subtrees on the next scan.
4. If `oc:checksums` is **absent** (older upload), fall back to size + etag
   heuristic; optionally upload with an `OC-Checksum: SHA1:<hex>` header so the
   server stores a checksum for next time.

**Generic WebDAV (no oc namespace):**
- Use **`size` + `etag`** as the only download-free signals (no server hash, no
  reliable mtime). Treat as a heuristic; if ambiguous (etag changed but you suspect
  metadata-only), offer an opt-in download+hash verify. This mirrors rclone's
  `--size-only` quick check, since true `--checksum` isn't possible here.

**Always compute the local side's hash locally** (stream once, cache it keyed by
local path+size+mtime so repeat scans are free) and compare to the server-provided
value. Never download the remote bytes purely to compare.

---

## Sources

Google Drive:
- Files resource reference (md5/sha1/sha256/size/modifiedTime/version/headRevisionId/quotaBytesUsed): https://developers.google.com/workspace/drive/api/reference/rest/v3/files
- Return specific fields (fields= FieldMask, list defaults): https://developers.google.com/workspace/drive/api/guides/fields-parameter
- Improve performance (partial response saves network/CPU/memory): https://developers.google.com/workspace/drive/api/guides/performance
- Retrieve changes (getStartPageToken / changes.list / nextPageToken / newStartPageToken): https://developers.google.com/workspace/drive/api/guides/manage-changes
- changes.getStartPageToken method: https://developers.google.com/workspace/drive/api/v2/reference/changes/getStartPageToken

WebDAV / Nextcloud / ownCloud:
- Nextcloud WebDAV basic (getcontentlength, getlastmodified, getetag, resourcetype, oc:id, oc:fileid, oc:permissions, oc:size, oc:checksums, namespaces): https://docs.nextcloud.com/server/latest/developer_manual/client_apis/WebDAV/basic.html
- ownCloud WebDAV API: https://owncloud.dev/apis/http/webdav/
- ownCloud OCS capabilities (supported checksum types / preferred upload type): https://doc.owncloud.com/server/10.16/developer_manual/core/apis/ocs-capabilities.html
- ownCloud Central — reading MD5/SHA1 hashes via WebDAV (oc:checksums + OC-Checksum format `SHA1:... MD5:... ADLER32:...`): https://central.owncloud.org/t/reading-hashes-md5-or-sha1-via-webdav/14348

rclone:
- rclone Google Drive (MD5/SHA1/SHA256 hashes, pre-2018 caveat, --drive-skip-checksum-gphotos): https://rclone.org/drive/
- rclone WebDAV (Nextcloud/ownCloud SHA1+MD5, version caveat, X-OC-Mtime, --webdav-vendor): https://rclone.org/webdav/
- rclone check (default server-side hash compare; --download; --size-only): https://rclone.org/commands/rclone_check/
- rclone global flags (--checksum: size+hash, for remotes without settable mtime): https://rclone.org/flags/
