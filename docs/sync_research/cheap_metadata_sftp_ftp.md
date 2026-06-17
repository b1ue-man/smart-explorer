# Cheap Remote File Metadata & Server-Side Hashing: SFTP and FTP/FTPS

Goal: for a Rust file-sync tool, find the cheapest data-wise way to decide whether a
remote file equals a local one **without downloading the bytes**, and document where a
**server-side hash** is actually feasible. Covers two protocol families: **SFTP** (over
SSH) and **FTP/FTPS**.

TL;DR up front:
- **SFTP**: cheap compare is effectively **size + mtime** from one `SSH_FXP_*STAT`. A true
  in-protocol server-side hash (`check-file`) exists only in an **expired draft** and is
  **not implemented by OpenSSH**; the practical hash path is **running `sha256sum`/`md5sum`
  over a separate SSH exec channel**, which needs shell access.
- **FTP/FTPS**: cheap compare is **size (`SIZE`/`MLSx`) + mtime (`MDTM`/`MLSx`)**. Hashes
  exist only as **non-standard `X*` commands** and a **never-standardized `HASH` draft**;
  support is per-vendor and unreliable.

---

## 1. SFTP (SSH File Transfer Protocol)

### 1.1 Cheap metadata: STAT/LSTAT/FSTAT → ATTRS

SFTP attributes are fetched with `SSH_FXP_STAT` / `SSH_FXP_LSTAT` (by path) and
`SSH_FXP_FSTAT` (by open handle). All three return an `SSH_FXP_ATTRS` reply carrying the
`ATTRS` structure (or `SSH_FXP_STATUS` on error). `STAT` follows symlinks; `LSTAT` does
not. (draft-ietf-secsh-filexfer-13, §8.5 "Retrieving File Attributes")

The `ATTRS` structure (draft-13 §7) begins with a `uint32 valid-attribute-flags`, and each
field is present only if its flag bit is set:

| Field | Type | Flag | Notes |
|---|---|---|---|
| `size` | `uint64` | `SSH_FILEXFER_ATTR_SIZE` (0x00000001) | bytes readable from file (§7.4) |
| `atime` | `int64` | `SSH_FILEXFER_ATTR_ACCESSTIME` (0x00000008) | last access |
| `mtime` | `int64` | `SSH_FILEXFER_ATTR_MODIFYTIME` (0x00000020) | last modified |
| `ctime` | `int64` | (attr-change) | metadata change |
| `(n)seconds` | `uint32` | `SSH_FILEXFER_ATTR_SUBSECOND_TIMES` (0x00000100) | optional nanoseconds added to each time |

Times are **seconds since 1970-01-01 UTC**, so default mtime precision is **1 second**.
**Subsecond (nanosecond) precision is only available when the peer sets
`SSH_FILEXFER_ATTR_SUBSECOND_TIMES`** (draft-13 §7.7); do not rely on it. (draft-13 §7.3-7.7)

Historical note on versions: in SFTP **v3** (the version OpenSSH actually speaks — see §1.2)
times are `uint32` seconds with no subsecond field at all; the richer 64-bit + subsecond
model is a later-protocol feature. So against OpenSSH you should assume **1-second mtime**.

**Cost**: one round-trip request/response per file; the reply is a few dozen bytes. This is
the cheapest signal available over SFTP. Pulling a whole directory's ATTRS in one go is
even cheaper via `SSH_FXP_READDIR`, which returns name + ATTRS per entry (so you usually do
**not** need a per-file STAT during a listing).

### 1.2 Server-side hashing in-protocol: the `check-file` extension — NOT in OpenSSH

The SFTP draft once defined a server-side checksum extension, `SSH_FXP_EXTENDED` with
extension names **`check-file-handle`** and **`check-file-name`**
(draft-ietf-secsh-filexfer-**09** §9.1.2 "Checking File Contents"). Request fields:
file handle or filename (string), a **hash algorithm list** (e.g.
`"md5,sha1,sha256,sha384,sha512,crc32"`), `start-offset` (uint64), `length` (uint64),
`block-size` (uint32). The response returns the `hash-algorithm-used` (string) followed by
the hash data (one digest per block, or a single digest if block-size is 0). It superseded
an even earlier `md5-hash` extension (draft-09 §9.1.1, dropped afterwards).

Critical reality check:
- **This extension only ever appeared in an expired Internet-Draft** (filexfer-09 / the
  separate `draft-ietf-secsh-filexfer-extensions-00`). It is **not in any RFC** and not in
  the final filexfer-13. (greenend SFTP versions table; filexfer-13 has no check-file.)
- **OpenSSH does NOT implement it.** OpenSSH's `PROTOCOL` file and the published OpenSSH
  SFTP extension list advertise only: `posix-rename@openssh.com`, `statvfs@openssh.com`,
  `fstatvfs@openssh.com`, `hardlink@openssh.com`, `fsync@openssh.com` (plus
  `lsetstat`, `limits`, `expand-path`, `copy-data`, `home-directory`, `users-groups-by-id`
  in newer master). **There is no `check-file`/hash extension.** OpenSSH's SFTP server
  therefore cannot hash a file for you over the SFTP channel.
- Only a few non-OpenSSH stacks ever shipped `check-file` (e.g. some commercial/Java SFTP
  servers list it as a planned/optional extension). For a tool talking to arbitrary
  servers, **assume it is absent.**

Conclusion: **you cannot count on an in-protocol server-side hash over SFTP**, because the
overwhelmingly dominant server (OpenSSH) doesn't offer one.

### 1.3 Practitioner workaround: run a hash binary over an SSH exec channel

The widely used trick (and what rclone's SFTP backend does) is to open a **separate SSH
`exec` session** and run `md5sum` / `sha1sum` / `sha256sum` on the remote shell, then parse
the hex digest. This is **not** SFTP — it's a second SSH channel — and requires the login to
have **shell/exec access**.

rclone SFTP backend specifics (rclone.org/sftp):
- "SFTP does not natively support checksums (file hash), but rclone is able to use
  checksumming if the same login has shell access, and can execute remote commands."
- `md5sum_command` / `--sftp-md5sum-command` and `sha1sum_command` / `--sftp-sha1sum-command`:
  by default rclone auto-probes commands (for MD5 it tries `md5sum`, `md5`, and
  `rclone md5sum`; SHA-1 similarly), picking the first usable one. You can pin an explicit
  command, or set it to `none`.
- `disable_hashcheck` / `--sftp-disable-hashcheck = true`: turn off checksumming entirely
  (equivalent to setting the hash commands / `hashes` to `none`).
- `shell_type` (`none`/`unix`/`powershell`/`cmd`): setting `none` disables checksumming **and**
  every other shell-exec-based feature.
- Caveat (verbatim spirit): disabling checksumming "may be required if you are connecting to
  SFTP servers which are not under your control, and to which the execution of remote shell
  commands is prohibited."

Reliability caveats: depends on a non-SFTP exec channel being permitted (many managed/jailed
SFTP-only accounts, e.g. `ForceCommand internal-sftp` / chroot, **forbid exec**); depends on
the right binary existing and matching across OSes (Windows/BusyBox differ); and it
**reads the whole file server-side** (CPU + disk I/O on the server), so it's "no download"
but not free for the server. Use it only when an exact-match guarantee is required.

### 1.4 SFTP verdict

- **Cheap compare signal**: **size + mtime** from one `SSH_FXP_STAT/LSTAT/FSTAT` (or, for a
  whole dir, from `READDIR`). Treat mtime as **1-second** precision against OpenSSH.
- **Server-side hash**: **not available in-protocol** on OpenSSH (no `check-file`). Only
  obtainable by **SSH-exec'ing `sha256sum`/`md5sum`**, which needs shell access and is
  unreliable on locked-down servers.
- **Recommendation**: default to **size + mtime** (with a tolerance window for mtime, since
  upload often resets it). Offer an **optional, opt-in** hash mode that uses an SSH exec of
  `sha256sum`, with capability detection at connect time and graceful fallback to size+mtime
  when exec is denied / binary missing.

---

## 2. FTP / FTPS

### 2.1 Cheap metadata: SIZE, MDTM, MLSD/MLST (RFC 3659)

RFC 3659 ("Extensions to FTP", Proposed Standard) adds `SIZE`, `MDTM`, `MLST`, `MLSD`:

- **`SIZE <path>`** (RFC 3659 §4): returns the number of octets that *would be transferred in
  the current TYPE/MODE/STRU*. **Major caveat**: the value depends on transfer type — in
  `TYPE I` (IMAGE/binary) it's the real byte count; in `TYPE A` (ASCII) it can differ (the
  RFC's example: 1830 octets binary vs 1942 ASCII due to CRLF expansion). **Always issue
  `TYPE I` before `SIZE`** so the number matches the on-disk byte length.
- **`MDTM <path>`** (RFC 3659 §3): last modification time, format `YYYYMMDDHHMMSS[.sss]`,
  **always in UTC/GMT**. Optional fractional seconds; in practice most servers return whole
  seconds, so treat as **1-second** precision. `MDTM`/`SIZE` "have been in wide use for many
  years" and are broadly available. (Setting time is via `MFMT`, RFC 3659 §3; some servers
  abuse a 2-arg `MDTM`.)
- **`MLSD <path>` / `MLST <path>`** (RFC 3659 §7): machine-readable listing with `fact=value;`
  pairs. Defined facts: `size`, `modify`, `create`, `type`, `unique`, `perm`, `lang`,
  `media-type`, `charset`. Servers **SHOULD** support at least `type, perm, size, unique,
  modify`. `modify` uses the same UTC `YYYYMMDDHHMMSS[.sss]` time format. So a **single
  `MLSD`** gives you size **and** mtime for every entry in a directory in one shot — the
  cheapest bulk signal. **There is no hash/checksum fact in MLSx.**

rclone FTP backend behavior (rclone.org/ftp): modtime is supported to **1-second** resolution
for major servers (ProFTPd, PureFTPd, VsFTPd, FileZilla). "If all the `MLSD`, `MDTM` and
`MFMT` extensions are present, rclone will use them together to provide precise time;
otherwise the times you see ... are those of the last file upload." VsFTPd needs
`writing_mdtm=true`. Check a server with `rclone backend features <remote>:`.

### 2.2 Server-side hashing: non-standard `X*` commands and the dead `HASH` draft

There is **no standardized FTP hashing command.** Two non-portable options exist:

1. **Vendor `X*` commands** (de-facto, undocumented in any RFC): `XCRC` (CRC32),
   `XMD5` (MD5), `XSHA`/`XSHA1`, `XSHA256`, `XSHA512`. Syntax e.g.
   `XCRC <filename> [SP] [EP]` (optional start/end byte range). Origin: GlobalSCAPE
   introduced `XCRC` (~2001); others followed inconsistently. Support is **per-vendor**:
   - **ProFTPD** via `mod_digest`: `XCRC`, `XMD5`, `XSHA1`, `XSHA256` (and the `HASH` command).
   - **Cerberus FTP Server**: `XCRC`, `XMD5`, `XSHA1`, `XSHA256`, `XSHA512`.
   - **Serv-U**: `XCRC` (others vary by version).
   - **vsftpd / pure-ftpd / FileZilla Server**: generally **no** built-in `X*` hash support.
2. **The `HASH` command** (`draft-bryan-ftp-hash`, latest -08, Oct 2010): a cleaner design —
   `FEAT` advertises e.g. `HASH SHA-256;SHA-512;SHA-1*;MD5` (the `*` marks the active algo),
   `OPTS HASH <algo>` selects it, a `RANG` command sets a byte range, and `HASH <path>`
   replies `213 SP <hashname> SP <hex> SP <path>`. **But the draft expired (2011-04-29) and
   was never published as an RFC** — it remained informational. Real-world support is
   sparse (notably ProFTPD `mod_digest`).

Reliability caveats: not in `FEAT` on most servers; algorithm naming/availability varies;
range semantics inconsistent across the `X*` family; FTPS adds TLS but doesn't change any of
this. So a hash is at best an **opportunistic optimization**, never a baseline assumption.

### 2.3 FTP verdict

- **Cheap compare signal**: **size + mtime**, ideally both from one **`MLSD`** (`size` +
  `modify` facts). Fallbacks: **`SIZE`** (after `TYPE I`) + **`MDTM`** when `MLSx` is absent.
- **Server-side hash**: **non-portable.** Only via vendor `X*` commands or the
  never-standardized `HASH` command, present on a minority of servers (ProFTPD/mod_digest,
  Cerberus, partial Serv-U). rclone's FTP backend doesn't support hashes at all.
- **Recommendation**: default to **size + mtime via `MLSD`** (one round-trip per directory),
  with `SIZE`+`MDTM` fallback. At connect, parse `FEAT`: if `HASH` or an `X*` command is
  advertised, **opportunistically** use it for an exact match; otherwise rely on size+mtime.

---

## 3. Cross-protocol recommendation for the sync tool

| | Cheapest reliable compare | Safe fallback | Server-side hash? |
|---|---|---|---|
| **SFTP** | `SSH_FXP_READDIR`/`STAT` → **size + mtime (1s)** | n/a (STAT is already the floor) | **No in-protocol** (OpenSSH lacks `check-file`); only SSH-exec `sha256sum` if shell allowed |
| **FTP/FTPS** | **`MLSD`** → **size + `modify` (1s)** | `SIZE` (after `TYPE I`) + `MDTM` | **Non-portable**; opportunistic `HASH`/`X*` if in `FEAT` |

Design guidance:
1. **Baseline equality = size + mtime** for both protocols, with an **mtime tolerance**
   (e.g. ±1-2 s) because uploads frequently rewrite mtime and precision is 1 s. Persist a
   local sync state (last-known size+mtime) so most files compare with a single metadata RTT.
2. **Capability-detect once per connection**: SFTP — probe whether a benign SSH exec
   (`echo`/`sha256sum --version`) succeeds; FTP — read `FEAT` for `MLST`/`HASH`/`X*`.
3. **Treat hashes as optional, opt-in confirmation**, used only when (a) the server can
   provide one cheaply and (b) the user wants exact-match guarantees. Never make a hash a
   precondition for normal compares — it requires reading the whole file server-side and is
   frequently unavailable.
4. Prefer **bulk listing** (`READDIR` / `MLSD`) over per-file metadata calls to minimize RTTs.

---

## Sources

SFTP / SSH:
- draft-ietf-secsh-filexfer-13 (final): attributes & STAT/LSTAT/FSTAT — https://datatracker.ietf.org/doc/html/draft-ietf-secsh-filexfer-13 (§7 ATTRS, §7.7 subsecond times, §8.5 stat ops)
- draft-ietf-secsh-filexfer-09 (check-file extension, §9.1.2; md5-hash §9.1.1) — https://datatracker.ietf.org/doc/html/draft-ietf-secsh-filexfer-09
- greenend SFTP versions table (md5-hash & check-file history) — https://www.greenend.org.uk/rjk/sftp/sftpversions.html
- OpenSSH PROTOCOL (extension list; no check-file) — https://github.com/openssh/openssh-portable/blob/master/PROTOCOL
- OpenSSH SFTP extensions list (sftp.net) — https://www.sftp.net/spec/openssh-sftp-extensions.txt
- rclone SFTP backend (md5sum_command, sha1sum_command, disable_hashcheck, shell_type) — https://rclone.org/sftp/

FTP / FTPS:
- RFC 3659 Extensions to FTP (SIZE §4, MDTM §3, MLST/MLSD §7) — https://www.rfc-editor.org/rfc/rfc3659.html
- draft-bryan-ftp-hash-08 (HASH command; XCRC/XMD5/XSHA* appendix; expired, never an RFC) — https://www.ietf.org/archive/id/draft-bryan-ftp-hash-08.html and https://datatracker.ietf.org/doc/html/draft-bryan-ftp-hash
- ProFTPD mod_digest (XCRC/XMD5/XSHA1/XSHA256 + HASH) — http://www.proftpd.org/docs/contrib/mod_digest.html
- Cerberus FTP supported commands (XCRC/XMD5/XSHA1/XSHA256/XSHA512) — https://support.cerberusftp.com/hc/en-us/articles/115001975970-Supported-FTP-Commands
- rclone FTP backend (no hashes; MDTM/MLSD/MFMT modtime, 1s) — https://rclone.org/ftp/
