# QUIC (iroh/noq) transport windows and SFTP (russh-sftp) pipelined reads

Checked 2026-09-28 against local crate sources under
`/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/` (read via `sudo -n`).

Versions (pinned in `native/Cargo.toml:38-40,95`): iroh 1.0.1, **noq 1.0.1**, **noq-proto 1.0.1**
(iroh's fork of quinn/quinn-proto — `repository = "…/n0-computer/noq"`, noq-1.0.1/Cargo.toml:31;
"builds on top of noq-proto", noq-1.0.1/src/lib.rs:1-7), russh 0.61.2, russh-sftp 2.3.0.

Project files read (as scoped): `native/src/share/core/keepalive.rs` (`iroh_transport_config()`
only), `native/src/sftp/core/posix_rename.rs` (full), `native/src/sftp/core/backend.rs`
(`open_session_channel` only).

Two unrelated stacks: (A) QUIC transport flow control for iroh peer shares, (B) SFTP
application-layer request pipelining over an SSH channel. Neither affects the other.

---

## A. iroh 1.0.1 / noq QUIC transport windows

### A.1 Current project baseline

`keepalive.rs:57-70` (`iroh_transport_config()`) only calls `.max_idle_timeout`,
`.keep_alive_interval`, `.default_path_keep_alive_interval`,
`.max_concurrent_bidi_streams(VarInt::from_u32(64))`, `.max_concurrent_uni_streams(VarInt::from_u32(0))`.
It never calls `.stream_receive_window` / `.receive_window` / `.send_window`, so those three sit at
whatever `QuicTransportConfigBuilder::new()` inherits from `noq_proto::TransportConfig::default()`
(A.4) — windows sized for "100 Mbps, 100 ms RTT" — while `max_concurrent_bidi_streams` is already
narrowed from noq-proto's default of 100 down to 64.

### A.2 `QuicTransportConfigBuilder` — exact signatures (iroh-1.0.1/src/endpoint/quic.rs)

Wraps `noq::TransportConfig` (`pub struct QuicTransportConfigBuilder(noq::TransportConfig)`, :92);
consuming-builder style; entry `QuicTransportConfig::builder()` (:134) → private `::new()` (:152,
see A.5) → `.build(self) -> QuicTransportConfig` (:166).

```rust
pub fn max_concurrent_bidi_streams(mut self, value: VarInt) -> Self   // :176
pub fn max_concurrent_uni_streams(mut self, value: VarInt) -> Self    // :182
pub fn max_idle_timeout(mut self, value: Option<IdleTimeout>) -> Self // :211
pub fn stream_receive_window(mut self, value: VarInt) -> Self         // :224
pub fn receive_window(mut self, value: VarInt) -> Self                // :235
pub fn send_window(mut self, value: u64) -> Self                      // :246  (u64, NOT VarInt)
pub fn send_fairness(mut self, value: bool) -> Self                   // :261
pub fn persistent_congestion_threshold(mut self, value: u32) -> Self  // :268
pub fn keep_alive_interval(mut self, value: Duration) -> Self         // :365 (wraps Some() internally)
pub fn congestion_controller_factory(
    mut self, factory: Arc<dyn noq_proto::congestion::ControllerFactory + Send + Sync + 'static>,
) -> Self                                                              // :419
pub fn enable_segmentation_offload(mut self, enabled: bool) -> Self    // :437 (GSO, default true)
```

Each setter forwards 1:1 to the same-named method on the wrapped `noq::TransportConfig`; doc
comments are copied verbatim from transport.rs (e.g. :216-223 ≡ transport.rs:120-128).

Congestion controllers in `noq_proto::congestion` (congestion.rs:11-13): `Cubic`/`CubicConfig`
(**default**, transport.rs:581), `NewReno`/`NewRenoConfig`, `Bbr3`/`Bbr3Config`, built via
`ControllerFactory::build(self: Arc<Self>, now: Instant, current_mtu: u16) -> Box<dyn Controller>`
(congestion.rs:143). This is orthogonal to flow control: the congestion window
(`ControllerMetrics::congestion_window`, congestion.rs:131) is a separate, self-tuning cap on
in-flight bytes — raising `send_window`/`receive_window` removes the flow-control ceiling only, it
does not disable congestion control.

### A.3 `VarInt` (noq-proto-1.0.1/src/varint.rs)

```rust
pub struct VarInt(pub(crate) u64);                             // :14, values < 2^62
pub const MAX: Self = Self((1 << 62) - 1);                     // :18 = 4_611_686_018_427_387_903
pub const MAX_SIZE: usize = 8;                                 // :20
pub const fn from_u32(x: u32) -> Self                          // :23  infallible, const fn
pub fn from_u64(x: u64) -> Result<Self, VarIntBoundsExceeded>  // :28  Err iff x >= 2^62
pub const unsafe fn from_u64_unchecked(x: u64) -> Self          // :41  caller ensures < 2^62
pub const fn into_inner(self) -> u64                            // :46
```

`max_concurrent_{bidi,uni}_streams`/`stream_receive_window`/`receive_window` take `VarInt`;
`send_window` takes plain `u64`.

### A.4 `noq_proto::TransportConfig::default()` (transport.rs:544-583)

Doc rationale (transport.rs:17-28): "…data window sizes can be tuned for a particular expected
round trip time, link capacity, and memory availability… **The default configuration is tuned for a
100Mbps link with a 100ms round trip time.**"

```rust
const EXPECTED_RTT: u32 = 100;                    // ms
const MAX_STREAM_BANDWIDTH: u32 = 12_500_000;     // bytes/s (100 Mbps / 8)
const STREAM_RWND: u32 = MAX_STREAM_BANDWIDTH / 1000 * EXPECTED_RTT;  // bandwidth-delay product
```

| Field | Default | Source |
|---|---|---|
| `max_concurrent_bidi_streams` / `_uni_streams` | `100` / `100` | :553-554 |
| `max_idle_timeout` | `Some(30_000 ms)` (RFC 9308 §3.2) | :555-556 |
| `stream_receive_window` | `STREAM_RWND` = **1,250,000 B** (~1.19 MiB) | :557 |
| `receive_window` | `VarInt::MAX` (unbounded aggregate) | :558 |
| `send_window` | `8 * STREAM_RWND` = **10,000,000 B** (~9.54 MiB) | :559 |
| `send_fairness` | `true` | :560 |
| `datagram_receive_buffer_size` | `Some(STREAM_RWND)` | :576 |
| `datagram_send_buffer_size` | `1_048_576 B` | :577 |
| `congestion_controller_factory` | Cubic | :581 |

Memory-scaling rule, quoted (transport.rs:81-82, repeated quic.rs:107-109,174-175):
> "Worst-case memory use is directly proportional to `max_concurrent_bidi_streams *
> stream_receive_window`, with an upper bound proportional to `receive_window`."

With the project's `max_concurrent_bidi_streams = 64` and a raised `stream_receive_window` of N
bytes, worst case is `64 * N` if all 64 streams are simultaneously full and unread, capped above by
`receive_window`. Leaving `receive_window` at `VarInt::MAX` doesn't itself inflate memory; it just
stops being an extra ceiling below `64 * N`. When raising `stream_receive_window` well past ~1.19
MiB, consider also bounding `receive_window` explicitly (e.g. a small multiple of the new
`stream_receive_window`) instead of leaving it unbounded.

### A.5 iroh-specific overrides in `QuicTransportConfigBuilder::new()` (quic.rs:152-160)

```rust
cfg.keep_alive_interval(Some(HEARTBEAT_INTERVAL));              // Some(5s)
cfg.default_path_keep_alive_interval(Some(HEARTBEAT_INTERVAL)); // Some(5s)
cfg.default_path_max_idle_timeout(Some(PATH_MAX_IDLE_TIMEOUT)); // Some(15s)
cfg.max_concurrent_multipath_paths(MAX_MULTIPATH_PATHS);        // 8
cfg.max_remote_nat_traversal_addresses(MAX_QNT_ADDRESSES);      // 32
cfg.server_handshake_migration(true);
```

Constants (iroh-1.0.1/src/socket.rs): `HEARTBEAT_INTERVAL = 5s` (:109, same value the project's own
`IROH_CONNECTION_KEEPALIVE_INTERVAL` uses — no conflict); `PATH_MAX_IDLE_TIMEOUT = 15s` (:117, "3x
HEARTBEAT_INTERVAL… WiFi reconnect 2-5s, cellular handoff 2-10s"); `MAX_MULTIPATH_PATHS = 8` (:137);
`MAX_QNT_ADDRESSES = 32` (:145).

None of this touches the window/stream-count fields. The struct doc (quic.rs:107-116) warns about
four *other* methods only: "In iroh, the config has some specific default values that make iroh's
holepunching work well with QUIC multipath. Adjusting those settings may cause suboptimal usage" —
naming `default_path_keep_alive_interval`, `default_path_max_idle_timeout`,
`max_concurrent_multipath_paths`, `max_remote_nat_traversal_addresses`. `stream_receive_window` /
`receive_window` / `send_window` / `max_concurrent_bidi_streams` are not among the warned-about
settings.

### A.6 Direction: which side's config limits which throughput

Doc text, quoted exactly:
- `stream_receive_window` (transport.rs:120-128): "Maximum number of bytes **the peer** may transmit
  without acknowledgement on any one stream before becoming blocked… set to at least the expected
  connection latency multiplied by the maximum desired throughput."
- `receive_window` (transport.rs:139-146): "Maximum number of bytes **the peer** may transmit across
  all streams of a connection before becoming blocked."
- `send_window` (transport.rs:148-155): "Maximum number of bytes **to transmit to a peer** without
  acknowledgment… upper bound on memory when communicating with peers that issue large amounts of
  flow control credit."

QUIC flow control is receiver-advertised, per direction (same model as upstream quinn/RFC 9000 §4).
`stream_receive_window`/`receive_window` are configured on the **receiving** side and bound how fast
the **remote peer** may send **to it** — they cap this side's inbound/download ceiling. `send_window`
is a local memory cap on this side's own unacked bytes; achievable outbound/upload throughput is
`min(local send_window, remote peer's advertised stream_receive_window/receive_window)`. Since both
Smart Explorer peers run the same `iroh_transport_config()`, raising all three symmetrically raises
the ceiling both ways; raising them on only one peer only raises that peer's inbound ceiling and its
own send-side memory budget, not what the unmodified peer will accept from it.

---

## B. russh-sftp 2.3.0 pipelined reader

### B.1 `RawSftpSession` (russh-sftp-2.3.0/src/client/rawsession.rs)

```rust
pub fn new<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(stream: S) -> Self   // :149
pub fn new_with_config<S: ...>(stream: S, cfg: Config) -> Self                     // :156
pub fn set_timeout(&self, secs: u64)                                                // :178
pub fn set_limits(&mut self, limits: Limits)                                        // :183  <- &mut self
pub async fn init(&self) -> SftpResult<Version>                                     // :238  {version:u32, extensions:HashMap<String,String>}
pub async fn open<T: Into<String>>(&self, filename: T, flags: OpenFlags, attrs: FileAttributes) -> SftpResult<Handle> // :247
pub async fn close<H: Into<String>>(&self, handle: H) -> SftpResult<Status>          // :282
pub async fn read<H: Into<String>>(&self, handle: H, offset: u64, len: u32) -> SftpResult<Data> // :324
pub async fn write<H: Into<String>>(&self, handle: H, offset: u64, data: Vec<u8>) -> SftpResult<Status> // :351
pub async fn limits(&self) -> SftpResult<LimitsExtension>                           // :679
pub fn close_session(&self) -> SftpResult<()>                                       // :230 (also Drop, :745)
```

`Config` (client/mod.rs:29-45): `max_packet_len: u32` (default `262_144` = 256 KiB),
`max_concurrent_writes: usize` (default `8`), `request_timeout_secs: u64` (default `10`).
`Handle{id:u32, handle:String}` (protocol/handle.rs:5-8). `Data{id:u32, data:Vec<u8>}`
(protocol/data.rs:5-9). `Limits{packet_len,read_len,write_len,open_handles:Option<u64>}`
(rawsession.rs:96-102; server's `0` → `None`/unlimited via `From<LimitsExtension>`, :104-113).

**Concurrent/pipelined reads — safe.** `request()`/`send()` (:187-223) assign a fresh id per call
via `next_req_id: AtomicU32::fetch_add` (:225), insert a fresh `oneshot::Sender` into
`requests: Arc<DashMap<Option<u32>, oneshot::Sender<...>>>` keyed by that id, push the serialized
packet onto a shared `mpsc::UnboundedSender<Bytes>`, then await only that call's own
`oneshot::Receiver`. A background task from `client::run()` (client/mod.rs:79-121) dispatches each
reply to the matching sender via `SessionInner::reply()` (:37-58), looked up by id regardless of
arrival order. Since every method but `set_limits` takes `&self` over thread-safe primitives, **N
calls to `.read(handle, offset_i, len)` may be in flight simultaneously** (multiple tokio tasks, or
one task driving `FuturesUnordered`/`join_all`) and may complete **out of order** — replies are
correlated by id, not response order, which is exactly what an explicit-offset pipelined reader
needs. Call `set_limits` once up front, before sharing the session across concurrent readers.

**Send + Sync.** Every field (:119-126: `mpsc::UnboundedSender`, `Arc<DashMap<...>>`, two
`AtomicU32`/`AtomicU64`, a `Copy` `Limits`) is `Send + Sync`, with no manual `unsafe impl` overriding
the auto traits — so `RawSftpSession` is automatically `Send + Sync` and can be wrapped in `Arc` and
driven from several tokio tasks, matching how `posix_rename.rs:31` builds one per short-lived
channel.

### B.2 `OpenFlags` — read-only open (protocol/open.rs:6-18)

```rust
pub struct OpenFlags(u32);  // bitflags
const READ = 0x01; const WRITE = 0x02; const APPEND = 0x04;
const CREATE = 0x08; const TRUNCATE = 0x10; const EXCLUDE = 0x20;
```

Read-only open is just `OpenFlags::READ` (compare `backend.rs:282`'s
`OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE` for the bit-or pattern already in use).

### B.3 EOF signaling

`StatusCode` (protocol/status.rs:7-41): `Ok=0`, **`Eof=1`** ("for SSH_FX_READ it means that no more
data is available in the file"), `NoSuchFile=2`, `PermissionDenied=3`, `Failure=4`, `BadMessage=5`,
`NoConnection=6`/`ConnectionLost=7` (client-only pseudo-errors), `OpUnsupported=8`.

A `read()` past EOF returns `Packet::Status{status_code: Eof, ..}`, turned by `into_with_status!`
into `Err(Error::Status(status))` (`impl From<Status> for Error`, client/error.rs:31-35). The
high-level `File::poll_read` (client/fs/file.rs:172-176) treats exactly this as end-of-stream:

```rust
Err(Error::Status(status)) if status.status_code == StatusCode::Eof => Ok(None),
```

A pipelined reader should apply the same match on each outstanding read's result — `Eof` means "no
more data at/after this offset"; every other `Err` is a real error.

### B.4 Safe read chunk size (mirrors `File::poll_read` exactly)

`file.rs:22`: `const READ_OVERHEAD_LENGTH: u32 = 9;` (`SSH_FXP_DATA` header: type(1)+id(4)+len(4)).

```rust
let max_read_len = features.limits
    .and_then(|l| l.read_len)
    .unwrap_or_else(|| features.max_packet_len.saturating_sub(READ_OVERHEAD_LENGTH)) as usize;
let len = usize::min(buf.remaining(), max_read_len);  // buf.remaining() is poll_read-buffer-specific
```

Use the server's advertised `limits@openssh.com` `read_len` if present, else `max_packet_len - 9` =
**262,135 bytes** with the crate's default `max_packet_len` (262,144). A pipelined reader that owns
its own buffers should request exactly this many bytes per in-flight READ (shorter only for the
final, EOF-bounded chunk) — the `buf.remaining()` clause is specific to `AsyncRead::poll_read`.

### B.5 Discovering the server's `limits@openssh.com` values

`extensions.rs:3`: `pub const LIMITS: &str = "limits@openssh.com";`. `LimitsExtension`
(extensions.rs:21-25): `{max_packet_len,max_read_len,max_write_len,max_open_handles:u64}` (`0` =
unlimited). Sequence used by `SftpSession::new()` (client/session.rs:35-68), to replicate manually
on a hand-built `RawSftpSession` (which bypasses `SftpSession`):

1. `session.init().await?` → `Version{version, extensions}`.
2. Check `has_extension(extensions::LIMITS, "1")` on `Version.extensions` (same map
   `posix_rename.rs:40` already checks for `posix-rename@openssh.com`).
3. If present: `let limits = Limits::from(session.limits().await?); session.set_limits(limits);`
   use `limits.read_len` in the B.4 formula; clamp `max_packet_len = min(server's, Config's)`
   (session.rs:67-68).
4. If absent: fall back to `Config.max_packet_len - 9`.

### B.6 russh 0.61.2 — opening the "sftp" subsystem channel

Same two calls `posix_rename.rs:27` and `backend.rs`'s `open_session_channel` (:99-129) already use:

```rust
// russh-0.61.2/src/client/mod.rs:675 — client::Handle
pub async fn channel_open_session(&self) -> Result<Channel<Msg>, crate::Error>
// Doc: "…returns Some(..) immediately if the connection is authenticated, but the channel only
// becomes usable when it's confirmed by the server."

// russh-0.61.2/src/channels/mod.rs:248 — Channel<Msg>
pub async fn request_subsystem<A: Into<String>>(&self, want_reply: bool, name: A) -> Result<(), Error>
// Doc: "Request the start of a subsystem with the given name."
```
Used as `channel.request_subsystem(true, "sftp")` (posix_rename.rs:27), then `channel.into_stream()`
feeds `RawSftpSession::new(stream)` (posix_rename.rs:31).

**Opening while another SFTP channel is busy is fine.** SSH (RFC 4254) multiplexes any number of
channels over one authenticated transport; `channel_open_session` sends its open request over an
internal `mpsc` and awaits its own confirmation independently of any other channel — there is no
connection-wide "one SFTP channel at a time" lock. This is why `posix_rename.rs`'s module doc
(lines 1-6) already opens "a short-lived second SFTP subsystem channel of the same SSH connection"
alongside the long-lived main `SftpSession` channel. A third, dedicated channel purely for pipelined
reads can coexist with both, reusing `backend.rs`'s retry-once-on-dead-generation pattern (:99-129).

### B.7 russh 0.61.2 — client window, packet size, channel buffer (added 2026-09-28, Block D)

`client::Config` (client/mod.rs:2023-2048), defaults (:2050-2068): `window_size: 2097152` (2 MiB),
`maximum_packet_size: 32768`, `channel_buffer_size: 100`, `keepalive_interval: None`,
`keepalive_max: 3`, `nodelay: false`. All three are per `Config`, i.e. per SSH session, and apply to
**every** channel opened on it (SFTP subsystem channels and exec channels alike):

- **Announced at open.** `channel_open_generic` (client/session.rs:10-45) sends
  `config.window_size` as the initial window and `config.maximum_packet_size` as the largest packet
  the server may send on the channel.
- **Refilled on receipt, not on consumption.** On every `CHANNEL_DATA` (client/encrypted.rs:453-472)
  the session loop calls `adjust_window_size(channel, data, target = config.window_size)`
  (:458; session.rs:285-317): the remaining window is reduced by the packet and, once it drops below
  `target / 2` (session.rs:301), a `WINDOW_ADJUST` tops it up to `target`. This happens before the
  data is handed to the channel, so the SSH window limits throughput to `window_size / RTT` per
  channel but never bounds memory.
- **Per-channel buffer is the only backpressure.** Each session channel gets a bounded
  `mpsc::channel(channel_buffer_size)` (client/mod.rs:676). The session loop forwards data with
  `chan.send(ChannelMsg::Data{..}).await` (encrypted.rs:470): when one channel's buffer is full the
  **whole session loop waits** — every other channel of the connection stalls (head-of-line). A
  buffer of `window_size / maximum_packet_size` messages holds one full window of full-size packets.
- **Packet size.** `connect_stream` logs an error if `maximum_packet_size > 65535`
  (client/mod.rs:1005-1010); OpenSSH itself uses 32 KiB session packets (below).
- **Refused channel.** `wait_channel_confirmation` (client/mod.rs:571-599) turns
  `SSH_MSG_CHANNEL_OPEN_FAILURE` into `Err(russh::Error::ChannelOpenFailure(reason))` (:594-596);
  `russh::ChannelOpenFailure` = `AdministrativelyProhibited=1 | ConnectFailed=2 |
  UnknownChannelType=3 | ResourceShortage=4 | Unknown=0` (lib_inner.rs:405-422, re-exported at the
  crate root via `include!("lib_inner.rs")`).
- `request_subsystem(want_reply, name)` only sends the request (channels/mod.rs:249-259); a refusal
  shows up later as a failing SFTP `init()`.

### B.8 russh-sftp 2.3.0 — raw session details for a pipelined reader/writer

- **Per-request timeout.** `RawSftpSession::send`/`request` (rawsession.rs:187-223) awaits each reply under
  `runtime::timeout(timeout_secs)` (client/runtime.rs, `tokio::time::timeout`, `Error::Timeout`),
  measured from the moment the request is queued; default 10 s, `set_timeout(&self, secs)`. A deep
  queue of READs therefore needs a bounded depth or a longer timeout.
- **`write_nowait` is `pub(crate)`** (rawsession.rs:379-397): outside the crate a pipelined writer
  spawns `write(handle, offset, data)` futures itself.
- **High-level `File`** (client/fs/file.rs): `poll_write` keeps at most `max_concurrent_writes`
  (default 8) WRITEs in flight and sends `min(caller buffer, write_len)` bytes per WRITE
  (file.rs:261-294) — with `io::copy`'s 8 KiB buffer that is 64 KiB in flight. `poll_flush` drains
  the acks and then sends `fsync@openssh.com` when advertised (file.rs:296-325); `poll_shutdown`
  drains and sends CLOSE (file.rs:327-356); `Drop` sends CLOSE without waiting (file.rs:135-143).
  `File::new` and `Features` are `pub(crate)`, so a raw channel cannot reuse `File`.
- **Handle limit.** `open`/`opendir` fail locally with `Error::Limited("handle limit reached")`
  once `limits.open_handles` handles are open (rawsession.rs:247-258), only after `set_limits`.
- **Extensions.** `extensions::{LIMITS = "limits@openssh.com", FSYNC = "fsync@openssh.com"}`
  (extensions.rs:3-6); `RawSftpSession::fsync(handle)` sends the extension (rawsession.rs:710-721).
- **Drop.** `impl Drop for RawSftpSession` calls `close_session()` (rawsession.rs:745-749), which
  sends an empty frame that makes the writer task shut the channel stream down (client/mod.rs:114-117).

### B.9 OpenSSH server facts that bound SFTP pipelining (checked 2026-09-28)

Sources: `openssh-portable` master (`sftp-server.c`, `sftp-common.h`, `channels.h`,
`serverloop.c`) and `sshd_config(5)` on man.openbsd.org.

- `SFTP_MAX_MSG_LENGTH = 256*1024`; `SFTP_MAX_READ_LENGTH = SFTP_MAX_MSG_LENGTH - 1024` (261,120
  bytes); `process_read` clamps longer READs to it. `limits@openssh.com` reports max-packet 262,144,
  max-read 261,120, max-write 261,120 and max-open-handles `RLIMIT_NOFILE - 5` (0 if unknown).
- The VERSION reply advertises `posix-rename@openssh.com`, `fsync@openssh.com`, `limits@openssh.com`,
  `copy-data` (server-side copy between two open handles; read length 0 = until EOF) and others.
- `MaxSessions`: "maximum number of open shell, login or subsystem (e.g. sftp) sessions permitted
  per network connection … The default is 10." Above it, `server_input_channel_open` answers with
  reason `SSH2_OPEN_CONNECT_FAILED` and the text "open failed" — not `AdministrativelyProhibited`, so
  every `ChannelOpenFailure` of a session channel has to count as a refusal.
- Session channels use `CHAN_SES_PACKET_DEFAULT = 32 KiB` and `CHAN_SES_WINDOW_DEFAULT = 64 × 32 KiB`
  (2 MiB): an upload on one channel moves at most 2 MiB per round trip; more channels, more windows.
- Comparable client (rclone, rclone.org/sftp, checked 2026-09-28): `--sftp-concurrency` "maximum
  number of outstanding requests for one file", default 64; `--sftp-idle-timeout` default 1m0s
  empties the connection pool when nothing was returned to it for that long.

### B.10 OpenSSH `copy-data` — server-side copy between two handles (checked 2026-09-29)

Sources: `openssh-portable` master `PROTOCOL` §4.10 and `sftp-server.c`
(`process_extended_copy_data`, `process_extended`, `request_permitted`, the handler table and
`process_init`); release notes `openssh.org/txt/release-9.0` and the 10.4 notes.

- **Availability.** Added in OpenSSH 9.0 (2022-04-08: "sftp-server(8): support the "copy-data"
  extension to allow server-side copying of files/data"; `sftp` got `cp` in the same release).
  `process_init` advertises it unconditionally as `compose_extension(msg, "copy-data", "1")`.
- **Request** (`SSH_FXP_EXTENDED`, request name `copy-data`), fields after the name:
  `string read-from-handle, uint64 read-from-offset, uint64 read-data-length,
  string write-to-handle, uint64 write-to-offset`. russh-sftp's `RawSftpSession::extended(name,
  data)` appends `data` unchanged after the name (`Extended.data` uses `data_serialize`, a raw
  byte sequence without length prefix), so the client builds exactly these bytes (big-endian,
  string = uint32 length + bytes). The reply is only `SSH_FXP_STATUS`: it carries **no byte
  count**, so the copied length has to be read afterwards (FSTAT of the write handle).
- **Semantics** (`process_extended_copy_data`): both handles must be open file handles of the
  same `sftp-server` process (handles are per process, i.e. per SFTP channel), else FAILURE; the
  same handle, the same path or the same `st_dev`/`st_ino` is refused with FAILURE (PROTOCOL
  says INVALID_PARAMETER for identical handles; 10.4 added the inode check). It seeks the read
  handle to read-from-offset and the write handle to write-to-offset (not with `O_APPEND`), then
  copies synchronously through a 64 KiB buffer; `read-data-length` 0 means "until EOF". The
  remaining length shrinks *before* each read, so a fixed length that meets the end of the source
  answers `SSH2_FX_EOF` — unless the end falls inside the last 64 KiB step, where the final
  `if (read_len == 0) status = SSH2_FX_OK` turns it into OK. Until-EOF answers OK. No `fsync`.
  The server process answers nothing else on that channel while it copies.
- **Refusals.** An unknown extended request gets `SSH2_FX_OP_UNSUPPORTED` ("MUST"). A known one
  that `request_permitted` refuses — read-only mode (`copy-data` is a writing request) or the
  `-P`/`-p` deny/allow lists — gets `SSH2_FX_PERMISSION_DENIED`. File permissions themselves were
  already checked by the two OPENs.

Use in Smart Explorer (`sftp/core/copy_data.rs`, plan W1 `server_copy_to_stage`): only on a pool
channel whose VERSION offered `copy-data` "1"; source OPEN(READ) and stage OPEN(WRITE|CREATE|
EXCLUDE) on that channel, then ranges — 16 MiB first, afterwards the previous range's rate × 10 s
(a sixth of the channel's 60 s answer deadline, at least one 64 KiB server step) — up to
`size + 1`, so a grown source shows up as one extra byte like in the stream path; EOF ends the
ranges; FSTAT gives the copied length; CLOSE without fsync. Any other status answer to a range
closes both handles, removes the stage and streams — copies inside one server streamed before, so
a server's copy must not turn them into failures, and the stream path reports a real file problem
on the right side; OP_UNSUPPORTED and PERMISSION_DENIED are also remembered for the connection
(the request policy is per server). A source that does not open streams too. Errors for the
engine stay: a request without an answer (timeout, lost channel), a stage that cannot be created
(a taken name is `AlreadyExists`, detected by LSTAT because protocol 3 answers EEXIST with plain
FAILURE, so the engine tries another name) and a failing FSTAT, CLOSE or cleanup.

---

## Unresolved questions

- Whether `noq`/iroh exposes the *current* connection's peer-advertised transport parameters at
  runtime (to auto-tune `send_window` to what the remote actually granted) — not investigated; would
  need `endpoint/connection.rs`'s stats API, outside this task's read scope (`keepalive.rs` only).
- ~~Whether russh's own SSH-level channel flow-control window needs enlarging~~ — resolved in B.7:
  the client window (`Config::window_size`, 2 MiB default) caps each channel at `window / RTT` for
  downloads; it is refilled on receipt and never bounds memory.
- No numeric throughput model beyond noq-proto's "100 Mbps / 100 ms" rationale and the qualitative
  `bidi_streams * stream_receive_window` memory rule exists; concrete target numbers for a given
  higher-bandwidth/higher-latency link are an implementer sizing decision, not a crate default.
- `noq-udp` (pinned alongside `noq`/`noq-proto`) was not read — it's the UDP I/O/GSO layer, not the transport-config/flow-control API this task scoped in.
