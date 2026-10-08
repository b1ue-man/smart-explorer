# Share-Server Signaling and Relay

`se-share-server` is an untrusted rendezvous and relay server. On the signaling
listener it routes signed presence, room events, and direct-access envelopes.
By default it also starts an Iroh transport relay on the adjacent port; peers
still authenticate each other and encrypt filesystem traffic end to end, so the
relay sees routing metadata and ciphertext but not relation secrets, private
keys, file names, file contents, or export configuration.

## Transports

- New input `host[:port]` means TLS WebSocket (`wss://host:51820` when the port
  is omitted). Explicit `wss://host/path` uses port 443 when omitted.
- `https://host/path` in the app is treated as `wss://host/path`.
- `tcp://host:51820` uses newline-delimited TCP; `ws://host/path` and
  `http://host/path` use plaintext WebSocket. New input needs the explicit
  **Unverschlüsselt erlauben** setting or CLI `--allow-plaintext` flag.
- Existing stored addresses without a scheme retain their old TCP meaning and
  are rewritten as `tcp://host:port`; their status shows **⚠ unverschlüsselt**.
  A TLS failure never falls back to a plaintext endpoint. Old stored mixed
  lists retain their entries but try only TLS entries while any are present;
  new input cannot mix TLS and plaintext.
- Append `#sha256=<64 hex digits>` to pin a self-signed TLS certificate or its
  SubjectPublicKeyInfo. Colon-separated certificate fingerprints are accepted.
  TLS still verifies that the server holds the matching private key.

For native signaling on port `N`, the Iroh relay listens on `N + 1` by default
(`51821` for `51820`). Set `SE_IROH_RELAY_BIND` to choose another relay bind or
`SE_IROH_RELAY_DISABLE=1` to disable it. Clients can override the derived relay
URL with `SE_SHARE_RELAY_URL`. A WebSocket address with a path derives the
relay's origin for a reverse proxy; without a path it tries the adjacent port
and origin. TLS addresses derive HTTPS relays, including the server's pin;
HTTP relay overrides and peer-advertised HTTP relays need the same plaintext
permission. Both listeners must be reachable when relay fallback is needed.

Relay startup is fail-closed by default. An invalid relay bind, runtime setup
failure, or occupied relay port makes `se-share-server` exit nonzero instead of
appearing healthy with only signaling available. Signaling-only operation is
permitted only when `SE_IROH_RELAY_DISABLE=1` (or `true`) is set explicitly.

Multiple app endpoints can be separated with commas or semicolons:

```text
wss://share.example.com/se-share, wss://backup.example.com/se-share
```

The client status reports **verschlüsselt** or **⚠ unverschlüsselt**. Its Hello
keeps legacy fields but sends no local-address list, device name or fingerprint.
Presence announcements continue to carry the routing material peers need.

## Switching an existing plaintext address to TLS

An address saved as `tcp://host:51820` (or `ws://…`) works only with the
plaintext permission and is shown as **⚠ unverschlüsselt**. To encrypt it:

1. The server must terminate TLS on its signaling port: `--tls-cert`/`--tls-key`
   or `SE_SHARE_TLS_CERT`/`SE_SHARE_TLS_KEY` (see below). The certificate must
   name the host the clients enter; an IP address needs a certificate for that IP
   or a pinned self-signed certificate (`#sha256=…`).
2. Enter the same host as `wss://host:51820` (or simply `host`, which means
   exactly that). The desktop settings and the Android Share settings offer this
   address with one click ("Verschlüsselte Adresse übernehmen"), the terminal
   prints it in `se share server show` (`encrypted` line); then save.
3. A TLS failure never falls back to plaintext. Once every client uses `wss://`,
   `SE_SHARE_ALLOW_PLAINTEXT` can be removed from the server.

## Native TLS and Key Login

The server can terminate TLS for both signaling and the adjacent Iroh relay:

```text
se-share-server 0.0.0.0:51820 --tls-cert fullchain.pem --tls-key privkey.pem --state-file share-bindings.json
```

Equivalent environment variables are `SE_SHARE_BIND`, `SE_SHARE_TLS_CERT`,
`SE_SHARE_TLS_KEY` and `SE_SHARE_STATE_FILE`. Both TLS files are required;
invalid or mismatched files stop startup. Keep the private key readable only by
the server account. On new handshakes both listeners share a resolver that
reloads a changed, validated pair. An incomplete or invalid replacement keeps
the previous pair; existing connections continue to work.

A public listener without TLS requires `--allow-plaintext` or
`SE_SHARE_ALLOW_PLAINTEXT=1`; otherwise startup explains the required options
and fails. Loopback listeners also permit plaintext for a TLS reverse proxy.
With TLS configured, public plaintext connections remain disabled unless
explicitly allowed. TLS failures never trigger a plaintext retry.

New clients negotiate `key_login_v1`. Before registration the server sends a
fresh 16-byte nonce; the client signs a domain-separated digest of the nonce,
device ID and public key with its Iroh key. The same ten-second registration
deadline covers TLS, WebSocket upgrade, Hello and challenge response. Device
IDs and Direct lookups bind to the first proven key; only that key may replace
their records or send the owner's decisions. Room membership must carry the
registered device ID and proven node/key. Disconnect and unpublish remove live
presence without forgetting ownership.

New Direct watchers present an HMAC-derived relation proof against the owner's
stored hash. New Room members see only the partition with the same proof.
These proofs reveal neither the relation secret nor a way to derive it. Older
clients can use unbound entries and participate in mixed rooms, but cannot
take over proven bindings. Compatibility mode allows legacy watchers without
proofs and legacy room members across partitions. Use `--require-key-login`
or `SE_SHARE_REQUIRE_KEY_LOGIN=1` when every client must prove its key and
relation access.

`--state-file` keeps key bindings across restarts with atomic replacement and
restrictive creation permissions on Unix. Full binding tables refuse new owners
instead of expiring or evicting existing ones. Save failures refuse the new
registration/publication. Without a state file ownership is memory-only and
startup warns that restart protection is unavailable.

## Temporary Discoverability and Background Pairing

The current Share UI can publish the owner's Direct identity or one existing
Room as a temporary discovery offer. Five minutes is the UI default and 30
minutes is the maximum. The terminal does the same with
`se share discoverable` (`list` and `stop` for running offers); the daemon keeps
this device's own offers in its worker snapshot, so every client sees them
regardless of who reads the shared event stream, and it allows one running
offer per target. The server grants at most a five-minute
lease at a time; the client renews shorter leases only until the original local
deadline, so reconnects and renewals cannot silently extend the user's chosen
visibility window.

PIN input is consumed as its exact UTF-8 byte sequence. It is not trimmed,
normalized, parsed as a number, or stored durably. The suggested PIN contains
six random digits. PINs shorter than six characters or easy to guess require
**Unsichere PIN erlauben** (`--allow-weak-pin` in the CLI); an empty PIN is always
rejected. The 1,024-byte upper bound remains a resource ceiling. CLI publishing
without a PIN option generates and prints a random PIN; `--pin-stdin` or
`--pin-prompt` accepts chosen PINs without putting them in the process list.
An offer ends after its first successful pairing or after five unsuccessful
answered exchanges, with a visible warning. Once a connector proves the PIN,
other concurrent starts are cancelled and no new one is admitted for the offer.

Discovery lists Direct devices and Rooms separately. Their display aliases are
untrusted labels; the cryptographic exchange binds the offer kind and the
server-issued discovery/exchange identifiers. Pressing **Connect** starts an
OPAQUE-based password-authenticated exchange in the background. The rendezvous
server routes bounded pairing packets and can observe public offer metadata and
timing, but it never receives the PIN, relation secret, private key, or decoded
application bundle. Wrong PINs, altered packets, replayed identifiers, expired
offers, and target changes are rejected. An error after a relation was already
persisted is reported as **gekoppelt – Bestätigung fehlt**, with its installed
contact or room available to keep or revoke; it is not reported as if nothing
had been installed. Failed removal preserves that notice. Already handed-out
Room credentials cannot be recalled: the notice explains that the room must be
recreated if those credentials must stop working.

For Direct offers, both peers exchange their authenticated identity and current
relation material and persist the reciprocal contact/grant before either side
sends the commit packet that claims persistence. The connecting device opens
its own exports only with the explicit **Auch meine Freigaben für dieses Gerät
öffnen** choice (`shareBack` in the API). For a Room offer, the joining peer
persists the publisher's current Room material; the publisher already owns that
Room and does not create a synthetic Direct relation. Neither flow requires a
second click or approval on the publishing side. Cancel and expiry terminate
only the affected offer or exchange.

The server allows four concurrent exchanges per offer, with at most one from
each proven connector key (or source address for a legacy connector). Twelve
starts per minute are counted per connector and offer, so one connector cannot
consume another connector's attempt budget. Offers remain subject to the
server's global and per-client capacity limits. Legacy clients behind an
explicitly trusted proxy count by connection, since the proxy already enforces
original-client limits and its shared backend address is not an identity.

## Signaling Resource Limits

The public signaling listener uses bounded resources so unauthenticated
internet traffic cannot create unbounded threads, queues, or retained routing
state. It admits at most 256 connection workers globally and 16 per source,
then at most 128 registered clients globally and eight per source. IPv4 sources
are counted by address and IPv6 sources by `/64`, preventing cheap address
rotation inside one `/64`; an additional `/56` cap limits multiple such sources
in one IPv6 allocation. A proven key has four simultaneous registrations by
default. Under registration pressure proven clients displace unproven legacy
registrations, preferring those without subscriptions. New sockets are additionally token
bucket limited to 128/s with a 256 global burst and 16/s with a 32 per-source
burst before a worker is spawned. The HTTP/WebSocket upgrade and the first valid
`Hello`, including its key challenge, must finish inside one absolute
ten-second deadline; invalid and
control-frame traffic cannot extend it. Registered connections accept at most
128 messages/s and 2 MiB/s. The initial burst permits two full publication
waves (1,282 messages and about 20 MiB), including immediate relay-ready route
updates and legacy access requests; sustained excess closes the connection.
For WebSockets, the byte limit is charged after TLS but before
frame parsing, including upgrade bytes, frame headers, masks, control frames,
and fragmented or empty continuation frames.

Each TCP or WebSocket client has a nonblocking outbound queue capped at 32
messages and 2 MiB of serialized JSON, plus a five-second socket write deadline.
Each queued message is serialized through a bounded 256 KiB buffer. A full queue
or oversized server message drops that individual route instead of allowing a
sender to disconnect its target; an expired or failed socket write closes the
client so its routing state can be removed. Reconnecting clients republish and
retry their signed endpoint-owned envelopes through the normal lifecycle.
Applying a changed Share profile explicitly unwatches removed or disabled
auto-connect contacts and leaves removed or disabled auto-join rooms before
publishing the replacement configuration, so GUI/CLI removal does not leave
stale server-side presence behind.

Per registered client, the server retains at most 64 published direct IDs, 256
watches, and 64 room memberships; rooms contain at most 64 clients. Identifier,
presence, candidate, key, name, URL, signed-direct envelope, receipt, decision,
digest, and user-message lengths are validated before insertion or forwarding.
WebSocket messages and raw JSON lines are limited to 256 KiB, and empty watcher
keys are removed on unwatch/disconnect. These limits affect only rendezvous
availability: the server still does not derive authorization from signaling
state or retain a durable direct-access inbox.

Startup limits are positive integer environment values:

| Variable | Default |
| --- | ---: |
| `SE_SHARE_MAX_CONNECTIONS` | 256 |
| `SE_SHARE_MAX_CONNECTIONS_PER_SOURCE` | 16 |
| `SE_SHARE_MAX_CONNECTIONS_PER_NETWORK` | four times the source limit (64) |
| `SE_SHARE_MAX_CLIENTS` | 128 |
| `SE_SHARE_MAX_CLIENTS_PER_SOURCE` | 8 |
| `SE_SHARE_MAX_CLIENTS_PER_NETWORK` | four times the source limit (32) |
| `SE_SHARE_MAX_CLIENTS_PER_KEY` | 4 |

Network limits apply to IPv6 `/56` groups. WebSocket output wakes its owner
thread immediately through the I/O reactor instead of waiting for a polling
interval. Client frames and messages share the 256 KiB ceiling; one client
drain hands over at most 256 messages or roughly 4 MiB before processing them.

A TLS/WebSocket reverse proxy otherwise collapses every signaling connection
into the proxy's backend address. Set `SE_SHARE_TRUSTED_PROXY_IPS` to exact,
comma- or semicolon-separated proxy IPs (for example `127.0.0.1,::1`) to keep
the global ceilings while delegating only the per-source worker, registration,
and accept-rate buckets for those backend sockets. The proxy must then enforce
equivalent connection, handshake, and request-rate limits per original client.
Only list loopback/private proxy addresses under your control. The server does
not trust spoofable forwarded-IP headers. Once this exemption is enabled, the
backend signaling listener must be reachable only from those proxies (normally
by binding to loopback and/or firewalling the port); otherwise another host can
connect directly and receive the proxy exemption.

The adjacent Iroh relay admits at most 512 TCP connections globally and 64 per
socket source before it spawns TLS, HTTP, WebSocket, or Iroh authentication
work. IPv4 sources are counted by address and IPv6 sources by `/64`; these
permits remain held for established relay connections and are returned by RAII
on every exit path. Accepted sockets are also token-bucket limited before a
handler is spawned: 256/s with a global burst of 512, and 32/s with a burst of
64 per IPv4 address or IPv6 `/64`; the source-bucket cache is an LRU capped at
4096 entries. TLS, HTTP upgrade, WebSocket upgrade, ClientAuth, authorization,
and actor registration share one absolute 30-second establishment deadline.
After Iroh's challenge/proof handshake, a second admission gate allows at most
four connections per authenticated Endpoint ID and 512 authenticated relay
connections in total. The relay also requires that the Endpoint ID has
registered at this server's signaling. Legacy Hello public keys remain usable
because the Iroh handshake separately proves possession. A ten-minute bounded
grace list allows reconnecting signaling clients to keep their relay path.

A reverse proxy is one socket source from the relay's perspective, so its
backend listener can delegate its source limits when that bind IP is explicitly
listed in `SE_SHARE_TRUSTED_PROXY_IPS`; the 512-connection global cap remains.
The proxy must enforce the per-original-client connection and request limits.
Each relay connection is limited to 64 MiB/s of received ciphertext with
an 8 MiB burst. Each destination's outgoing packet queue is capped at 512
packets and 1 MiB of retained payload, with a shared 64 MiB payload ceiling for
all destination queues; byte reservations remain held through the bounded write
attempt and are returned on send, timeout, rejection, or disconnect. Peer-route
notification relationships are removed when either endpoint disappears, and
the public key cache holds at most 4096 entries. These are availability controls
only and do not weaken or replace peer authentication or end-to-end encryption.

## Sleeping Clients (Idle Keepalive)

A client that negotiates `idle_keepalive_v1` in `hello` (the Android app while
it is not visible) can announce that it sleeps. The server then keeps its
signaling connection alive without the client's 20-second heartbeat, so a phone
in Doze stays reachable for Direct-Share requests while it wakes only once per
keepalive interval. Older servers and clients never send these messages:

```text
Client -> Server  {"t":"set_idle","idle":true|false[,"keepalive_secs":N]}
Client -> Server  {"t":"keepalive_ack"}
Server -> Client  {"t":"idle_ack","idle":true|false,"keepalive_secs":K}
Server -> Client  {"t":"keepalive"}
```

- **Interval K** comes from `SE_SHARE_IDLE_KEEPALIVE_SECS` (default 180).
  Values outside 30-210 s are clamped with a startup notice; a non-numeric value
  stops the server. 210 s is the ceiling because a signed presence lives 300 s,
  desktops refresh theirs every 60 s, and a 30-second delivery margin remains.
  `set_idle` may propose a shorter interval (`keepalive_secs`, at least 30 s);
  the server uses the smaller value and reports it in `idle_ack`.
- **Idle:** the server ticks every K seconds, independent of inbound traffic.
  Each tick first delivers deferred presence refreshes, then `keepalive`. After
  a keepalive the client must send something (normally `keepalive_ack`) within
  60 s; otherwise the server closes the connection and removes its
  registrations like on any disconnect.
- **Not idle**, and every client without the capability: TCP and WebSocket
  connections close after 60 s without inbound data. A raw TCP line that arrives in pieces across the
  server's internal wake-ups is kept, not discarded.
- `set_idle` with `idle:false` first delivers everything deferred, then
  `idle_ack`. `set_idle` without the negotiated capability is ignored.

**Presence bundling.** Per idle connection the server remembers the route (Iroh
node ID, relay URL, candidates) and expiry of the last presence it sent for each
Direct lookup and each room member. A `direct_available` or `room_joined`
refresh waits for the next tick only when this connection already received a
presence for that key, the route is unchanged, and the copy it holds stays valid
at least 30 s beyond that tick (expiry capped at send time + 300 s and judged on
the wall clock). A newer refresh replaces a waiting one. Everything else is sent
at once: first announcements, route changes, refreshes of a nearly expired copy,
`direct_offline` and `room_left` (which also drop the waiting refresh for that
key), access requests, decisions, receipts, discovery, and pairing. Waiting
refreshes never retain more than the 2 MiB writer-queue budget; beyond it they
are sent immediately. Unwatching a lookup or leaving a room forgets its keys, so
a later watch or join starts with an immediate presence. With K = 180 s and a
desktop refreshing every 60 s, an idle phone receives one refresh per keepalive
and its copy never expires.

Idle connections count against the same connection-worker, per-source, and
registration limits as every other connection.

**Relay pings.** The Iroh relay pings each connection 15 s (plus 1-5 s jitter)
after the last received frame until the client has answered one ping, then
every K seconds (plus 1-5 s jitter). Every ping must be answered within a fixed
30 s. Desktop clients ping the relay themselves every 15 s, which restarts this
timer, so nothing changes for them; a sleeping phone is woken by the relay at
most once per K. A connection superseded by a newer connection of the same
Endpoint ID returns to the 15-second check, so a stale connection releases its
admission slot (four per Endpoint ID) quickly.

**Proxies.** Every reverse proxy or load balancer on the path must keep an idle
upgraded connection (signaling WebSocket and relay) open for at least K + 60 s
(240 s by default); the nginx example below sets 300 s. If a proxy's idle
timeout cannot be raised, lower K with `SE_SHARE_IDLE_KEEPALIVE_SECS` so that
K + 60 s stays below it.

## HTTPS / 443 Deployment

Alternatively, bind both backend listeners to loopback and terminate TLS in a
reverse proxy:

```text
SE_SHARE_TRUSTED_PROXY_IPS=127.0.0.1 se-share-server 127.0.0.1:51820 --state-file share-bindings.json
```

Caddy example:

```caddyfile
share.example.com {
    @signaling path /se-share
    reverse_proxy @signaling 127.0.0.1:51820
    reverse_proxy 127.0.0.1:51821
}
```

Nginx example:

```nginx
# http {} scope: bound the original public client before backend source collapse
limit_conn_zone $binary_remote_addr zone=se_share_conn:10m;
limit_req_zone $binary_remote_addr zone=se_share_req:10m rate=16r/s;

location /se-share {
    limit_conn se_share_conn 8;
    limit_req zone=se_share_req burst=32 nodelay;
    proxy_pass http://127.0.0.1:51820;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    # Idle phones are pinged every K s (default 180): keep >= K + 60 s.
    proxy_read_timeout 300s;
    proxy_send_timeout 300s;
}

location / {
    limit_conn se_share_conn 8;
    limit_req zone=se_share_req burst=32 nodelay;
    proxy_pass http://127.0.0.1:51821;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_read_timeout 300s;
    proxy_send_timeout 300s;
}
```

The Caddy snippet shows routing/TLS topology only; stock Caddy has no equivalent
per-client connection limiter. Put a rate-limiting WAF/load balancer in front of
it or use an audited limiter module before enabling the trusted-proxy exemption.
Also check that no Caddy, load-balancer, or CDN idle timeout on the path is
shorter than K + 60 s (see Sleeping Clients).

This makes both signaling and the encrypted Iroh relay reachable through the
same TLS hostname. Iroh still tries direct peer paths first. If the data relay is
disabled or unreachable, direct connections can continue to work, while peers
that need fallback remain unreachable and report that as connectivity rather
than as an access-request decision.

## Tracked Direct-Access Lifecycle

Adding a direct code does more than create a local contact. The requester first
persists a signed request with a stable UUID, then asks the Share worker to
deliver that exact envelope. The target persists and verifies it before showing
it as received. A valid new request is then automatically accepted unless the
exact identity or relation conflicts with local state or durable policy records
an ignore, rejection, revocation, or deletion tombstone; those cases are
automatically rejected and never overwritten. Accept, reject, and revoke remain
signed, revisioned decisions, and the other endpoint returns a signed receipt.
Pending envelopes remain in the endpoint's durable ledger and are retried after
a worker or app restart.

Once an accepted Direct pair is online, a bounded background repair exchanges
the missing reciprocal contact/grant and persists it on both endpoints before
reporting completion. This also upgrades earlier one-way relations when both
clients support the reciprocal protocol. It is idempotent across disconnects
and retries; explicit denial or identity/material conflict stops repair until
the corresponding local state changes. A peer without the capability keeps its
existing file session and is retried later rather than breaking connectivity.

The user-visible state deliberately has separate axes:

| Axis | Values | Meaning |
| --- | --- | --- |
| Request delivery | `queued`, `sent`, `server_queued`, `delivered`, `received`, `failed`, `expired` | Progress of the signed request envelope. Only `received` is backed by the target's signed request receipt. |
| Signaling route result (`relay.*`) | `forwarded`, `legacy_forwarded`, `target_offline`, or unconfirmed | What the rendezvous server did with the latest access envelope. `legacy_forwarded` means it translated the request for an online old client. This is not peer receipt or Iroh data-path status. |
| Decision | `pending`, `accepted`, `rejected`, `revoked`, `failed`, `expired` | Latest signed request decision and its revision. For an old peer, `effective_state` and `evidence=legacy_relation` separately report the verified legacy relation decision. |
| Decision delivery | `not_started`, `queued`, `sent`, `server_queued`, `delivered`, `received`, `failed`, `expired` | Progress of the signed decision envelope. On the deciding endpoint, `received` requires the peer's signed decision receipt. |
| Authorization | `active` or `inactive` | Whether the verified decision is currently projected into a local contact/grant. |
| Connectivity | offline/waiting/connecting/direct/relay/error state | Current data-path reachability. It is independent of authorization and signaling delivery. |

Consequently, `queued` means only "durably stored on this endpoint". A signaling
route ACK of `forwarded` means only that the server enqueued the envelope to a
currently connected compatible client writer. It does not mean that the peer
read, verified, or persisted it. `legacy_forwarded` means the server delivered
the compatibility request to an online old client and stops automatic duplicate
popups; that client cannot return a signed request receipt. Its HMAC-verified
relation decision is shown separately from the still-unconfirmed signed request
decision, while authorization reports whether access is actually active.
`target_offline` is likewise a retryable signaling observation, not a rejected
request. These route ACKs are distinct from connectivity such as
`connected_relay`, which describes an authenticated Iroh data session using the
encrypted transport relay.

The rendezvous server does not store a direct-access inbox. Durable outbox,
inbox, retry counters, signed envelopes, receipts, decisions, and grants live on
the endpoints. Replaying the same request keeps the same request ID and is
idempotent. Clients negotiate `tracked_direct_v1`; old fallback messages remain
visible with explicit legacy evidence because they cannot prove all signed
lifecycle states. A new server translates a new request for an online old
target only after the supplied legacy presence exactly matches the signed
requester identity and relation.

## Manage Requests and Grants

The GUI's **Teilen** view shows three durable sections:

- **Eingehende Anfragen**: request ID, requester identity, fingerprint,
  transport/receipt state, automatic policy decision, authorization, and
  explicit management actions for retained or older states. A normal verified
  new request needs no second confirmation. Local delete stops retries and
  persists a bounded replay tombstone; signed reject remains the peer-visible
  decision.
  Completed entries move into a collapsed history. An accepted incoming entry
  remains until its active grant is signed-revoked and the peer receipt arrives;
  this preserves the only durable revoke outbox. It can then be removed locally
  without revoking an independent grant.
- **Ausgehende Anfragen**: the same lifecycle state from the requester side and
  a retry action that reuses the request ID.
- **Autorisierte Geraete**: active grants and connectivity, with signed revoke
  for tracked grants. A legacy grant can only be disabled locally and is marked
  as such. **Geraet entfernen** (active) / **Eintrag loeschen** (inactive)
  delete the grant, its Exec grant and every request of that device completely
  and record the device under **Entfernte Geraete**.
- **Entfernte Geraete**: devices removed by the user. The record is what keeps
  a peer that still holds this device's Direct code from re-installing itself
  automatically: the reciprocal background repair answers `PolicyDenied` and
  an auto-accepted access request is rejected. Any deliberate pairing (PIN
  discovery, adding the Direct code, accepting a request) or **Erneut
  zulassen** clears the record. CLI: `se share grants delete <selector>`,
  `se share grants removed [--readmit <device>]`.

Removing a Direct contact (`Entfernen`, `se connections remove-peer`) runs one
profile transaction that deletes the contact, its relation secret, the grant
for its remote device, tracked and legacy requests of that device (with replay
tombstones while the bounded ledger has room) and writes the removed-device
record; schema version 8 adds `removed_direct_peers`. Outside the profile the
app also drops favourites and folder preferences under
`share://direct/<id>/…`, stops drive mounts of that peer, closes its tabs and
reports sync jobs that still reference it (they are kept and marked orphaned).

The terminal exposes the same ledger:

```text
se share request [--json]
se share request show [<selector>] [--json]
se share request accept [<selector>] [--fingerprint <assertion>] [--message <text>] [--json]
se share request reject [<selector>] [--fingerprint <assertion>] [--message <text>] [--json]
se share request retry [<selector>] [--json]
se share request delete [<selector>] [--json]
se share grants [--json]
se share grants revoke [<selector>] [--fingerprint <assertion>] [--message <text>] [--json]
se share grants exec [enable|disable] [<selector>] [--yes] [--json]
se exec [<peer-selector>] -- PROGRAM [ARGS...]
se exec [<peer-selector>] --shell COMMAND
se share exec [list|history] [--json]
se share exec show <selector> [--json]
se share exec cancel [<selector>] [--json]
se share status [--json]
se completions bash|zsh|fish|elvish|powershell
```

Bare `request`, `grants`, and `connections` commands list the corresponding
objects and expose selectors accepted by their action commands. If exactly one
request, retryable envelope, deletable history row, or active grant is eligible,
`show`, `accept`, `reject`, `retry`, `delete`, or `revoke` auto-selects it. With
multiple matches the command prints the exact usable selectors instead of
requiring hidden data. Optional fingerprint arguments are additional assertions
against the signed stored identity, never required input.
Request/status output reports delivery, relay result, peer receipts, decision
and revision, retry attempts/errors, authorization, and connectivity separately.
Profile mutations use bounded compare-and-swap retries, so normal concurrent
GUI/worker updates are rebased instead of overwriting each other.

An incoming request whose device ID matches an existing tracked or legacy
identity but whose public key, node ID, or fingerprint differs is an explicit
`identity_conflict`. It is shown in text and JSON, automatically rejected, and
excluded from grant projection. The GUI names the blocker; the CLI prints exact
revoke/reject/delete resolution commands. Reject and local delete remain
available so a conflict cannot strand an unmanageable inbox row. An
already active old identity must be revoked before the conflicting request can
be reconsidered; no key change silently inherits its authority.

Exec permission is separate from file access and default-denied for every exact
device identity. Enabling it grants unrestricted code execution as the account
running Smart Explorer; there is deliberately no command or path allowlist.
Windows uses a kill-on-close Job Object and Linux a transient systemd cgroup.
The CLI auto-selects the only eligible peer/grant/job, otherwise it prints the
valid choices and usable commands. Active and terminal execution state is
visible on both endpoints, and Disable/Cancel terminates the contained tree
before a terminal state is recorded.

Unauthenticated Iroh failures cannot grow the worker's diagnostics without
bound. FS/Exec handshake and stream failures are classified into a fixed set,
their detail is truncated, repeated failures are coalesced, and the service
event channel is bounded. A flood can therefore lose repetitive diagnostic
lines, but it cannot consume unbounded memory or suppress a later legitimate
connection once the consumer drains capacity.

The accepted direct code authorizes the owner's `Standard Direkt` export scope.
That scope is visible next to the code and can be changed before initiating the
pairing or while the service is online. Windows firewall setup is attempted
automatically;
if a normal rule fails, the app asks Windows for elevated firewall permission
through UAC.

## Local-Network Presence and Uplink Sharing

Paired Direct devices can find each other without the signaling server. The
worker announces `_se-share._udp.local.` over mDNS with TXT `v=1`,
`id=<16 hex>` (the first 8 bytes of `SHA-256("se-lan-presence-v1|" + node
id)`), `p4`/`p6` (bound Iroh UDP ports) and `up=0|1` (advisory "has own
internet"). Sightings whose id matches an accepted contact's pinned node become
`lan_candidates` on that contact (`ip:p4`, `[ip]:p6`, or `[fe80::x%<ifindex>]:p6`
once per local interface for link-local IPv6) for `LAN_PRESENCE_TTL_SECS = 150`.
Dialing merges them with a current server presence or synthesizes a presence
from the contact pins when the server is unreachable; the Iroh TLS node pin
and the relation session proof still authenticate every session, so the
announcement is routing evidence only. With no server configured the worker
runs in offline mode as long as LAN presence is enabled and accepted peers
exist. Every facility (mDNS socket, interface enumeration, settings file)
reports `nicht verfuegbar: <Grund>`; nothing is assumed.

Interfaces are classified from typed facts (Linux: sysfs, `/proc/net/route`,
`/proc/net/ipv6_route`, lease files or NetworkManager; Windows:
`GetAdaptersAddresses` including gateways and the DHCPv4 server):
`RouterLess` (up, no default gateway, no DHCP lease), `Uplink` (gateway plus
the platform's internet verdict when it has one), `Routed`, `Inactive`. A link
where a paired peer was seen never counts as uplink, so the receiving side of
a shared connection does not report "has internet" back.

Automatic uplink sharing is an opt-in (`lan_settings.json`:
`uplink_sharing_enabled`). The first activation runs a one-time setup:
Windows registers the on-demand Scheduled Task `Smart Explorer LAN-Uplink`
(`se.exe --lan-uplink-helper`, highest privileges, one UAC prompt) and makes
the `SharedAccess` service startable; Linux installs
`/etc/polkit-1/rules.d/49-smart-explorer-lan-uplink.rules` through `pkexec`
for `org.freedesktop.NetworkManager.settings.modify.system` and
`network-control` (without `pkexec` the calls still go through and polkit may
prompt on the desktop). The pure policy (`share/core/lan_uplink_policy.rs`)
starts sharing on a `RouterLess` link after 5 s of a paired peer announcing
`up=0` while this host has an `Uplink`, and stops 90 s after the peer vanished,
15 s after the uplink was lost, on `se share lan uplink stop`, when the
setting is disabled, or at daemon shutdown. Windows applies ICS
(`HNetCfg.HNetShare`, public/private) through the elevated helper, which
re-validates the request file (age, GUIDs, router-less private adapter);
Linux activates a dedicated NetworkManager profile `Smart Explorer LAN-Uplink
(<iface>)` with `ipv4.method=shared` (NAT + dnsmasq) and deletes it again on
stop. `lan_uplink_state.json` records the active session so a restarted daemon
reconciles it. Status: GUI **Teilen → LAN**, `se share lan [--json]`.

## Lifecycle Regression Guard

CI runs native Windows library and standalone-`se` tests plus
`native/test-share-lifecycle-e2e-windows.ps1` on a Windows runner. After the
cross-platform release build it downloads the exact staged Windows GNU
`se.exe` and `se-share-server.exe` and runs the same script again; release
publication depends on that exact-binary gate. The lifecycle
uses four isolated endpoint/credential profiles, a real relay, context-free
request and grant commands, and a real contained remote `cmd.exe`. It verifies
pending-inbox survival across a worker restart, offline acceptance delivery,
receipts, reject, pending deletion across two restarts, disable, signed revoke,
and history deletion. The cross-platform build runs
`native/test-share-lifecycle-e2e.sh` with the equivalent four isolated Linux
profiles and a real `se-share-server`. That scenario obtains every selector,
request, peer, grant, path, and Exec ID from earlier CLI output in the same run;
checks request receive/accept/reject/delete persistence; and exercises the full
installed-CLI Exec lifecycle: binary I/O, nonzero exit, output cap, timeout,
healthy idle beyond the heartbeat interval, Cancel, CLI disconnect, worker
`SIGKILL`, permission revoke, containment cleanup, and denied execution
afterward. It also proves that active accepted history cannot be erased, then
performs signed base-grant revoke, waits for its receipt, deletes the inactive
history context-free, and confirms denied file access.

The exact-candidate Linux gate also downloads the published `se` from v0.5.126
under its pinned SHA-256 and runs `native/test-share-mixed-version-e2e.sh`.
It proves new-to-old retry and acceptance plus old-to-new durable inbox,
context-free accept/reject/retry/revoke/delete, completion, and file access.
Every selector used by that scenario is obtained from earlier CLI output in the
same run.
