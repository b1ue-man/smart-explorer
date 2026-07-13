# Share-Server Signaling and Relay

`se-share-server` is an untrusted rendezvous and relay server. On the signaling
listener it routes signed presence, room events, and direct-access envelopes.
By default it also starts an Iroh transport relay on the adjacent port; peers
still authenticate each other and encrypt filesystem traffic end to end, so the
relay sees routing metadata and ciphertext but not relation secrets, private
keys, file names, file contents, or export configuration.

## Transports

- `host:51820` or `tcp://host:51820`: raw newline-delimited TCP signaling.
- `ws://host/path`: WebSocket signaling without TLS.
- `wss://host/path`: WebSocket signaling through TLS, intended for port 443.
- `https://host/path` in the app is treated as `wss://host/path`.

For raw TCP signaling on port `N`, the Iroh relay listens on `N + 1` by default
(`51821` for `51820`). Set `SE_IROH_RELAY_BIND` to choose another relay bind or
`SE_IROH_RELAY_DISABLE=1` to disable it. Clients can override the derived relay
URL with `SE_SHARE_RELAY_URL`. Both listeners must be reachable if encrypted
relay fallback is required.

Relay startup is fail-closed by default. An invalid relay bind, runtime setup
failure, or occupied relay port makes `se-share-server` exit nonzero instead of
appearing healthy with only signaling available. Signaling-only operation is
permitted only when `SE_IROH_RELAY_DISABLE=1` (or `true`) is set explicitly.

Multiple app endpoints can be separated with commas or semicolons:

```text
wss://share.example.com/se-share, share.example.com:51820
```

## Signaling Resource Limits

The public signaling listener uses fixed resource ceilings so unauthenticated
internet traffic cannot create unbounded threads, queues, or retained routing
state. It admits at most 256 connection workers globally and 16 per source,
then at most 128 registered clients globally and eight per source. IPv4 sources
are counted by address and IPv6 sources by `/64`, preventing cheap address
rotation inside one normal IPv6 allocation. New sockets are additionally token
bucket limited to 128/s with a 256 global burst and 16/s with a 32 per-source
burst before a worker is spawned. The HTTP/WebSocket upgrade and the first valid
`Hello` must both finish inside one absolute ten-second deadline; invalid and
control-frame traffic cannot extend it. Registered connections accept at most
128 messages/s and 2 MiB/s with the same-size burst before the offender is
closed. For WebSockets, the byte limit is charged on raw wire reads before
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
four connections per authenticated Endpoint ID.

A reverse proxy is one socket source from the relay's perspective, so its
backend connection budget must fit inside the 64-connection source ceiling and
the proxy must enforce a smaller per-original-client WebSocket connection
limit. Each relay connection is limited to 64 MiB/s of received ciphertext with
an 8 MiB burst. Each destination's outgoing packet queue is capped at 512
packets and 1 MiB of retained payload, with a shared 64 MiB payload ceiling for
all destination queues; byte reservations remain held through the bounded write
attempt and are returned on send, timeout, rejection, or disconnect. Peer-route
notification relationships are removed when either endpoint disappears, and
the public key cache holds at most 4096 entries. These are availability controls
only and do not weaken or replace peer authentication or end-to-end encryption.

## HTTPS / 443 Deployment

Run the server locally and terminate TLS in a reverse proxy:

```text
SE_SHARE_TRUSTED_PROXY_IPS=127.0.0.1 se-share-server 127.0.0.1:51820
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
}

location / {
    limit_conn se_share_conn 8;
    limit_req zone=se_share_req burst=32 nodelay;
    proxy_pass http://127.0.0.1:51821;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
}
```

The Caddy snippet shows routing/TLS topology only; stock Caddy has no equivalent
per-client connection limiter. Put a rate-limiting WAF/load balancer in front of
it or use an audited limiter module before enabling the trusted-proxy exemption.

This makes both signaling and the encrypted Iroh relay reachable through the
same TLS hostname. Iroh still tries direct peer paths first. If the data relay is
disabled or unreachable, direct connections can continue to work, while peers
that need fallback remain unreachable and report that as connectivity rather
than as an access-request decision.

## Tracked Direct-Access Lifecycle

Adding a direct code does more than create a local contact. The requester first
persists a signed request with a stable UUID, then asks the Share worker to
deliver that exact envelope. The target persists and verifies it before showing
it as received. Accept, reject, and revoke are signed, revisioned decisions;
the other endpoint returns a signed receipt for the decision. Pending envelopes
remain in the endpoint's durable ledger and are retried after a worker or app
restart.

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
  transport/receipt state, decision, authorization, and actions to accept after
  explicitly confirming the displayed fingerprint or to delete/reject without
  an unrelated confirmation hurdle. Local delete stops retries and persists a
  bounded replay tombstone; signed reject remains the peer-visible decision.
  Completed entries move into a collapsed history. An accepted incoming entry
  remains until its active grant is signed-revoked and the peer receipt arrives;
  this preserves the only durable revoke outbox. It can then be removed locally
  without revoking an independent grant.
- **Ausgehende Anfragen**: the same lifecycle state from the requester side and
  a retry action that reuses the request ID.
- **Autorisierte Geraete**: active grants and connectivity, with signed revoke
  for tracked grants. A legacy grant can only be disabled locally and is marked
  as such.

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
`identity_conflict`. It is shown in text and JSON but excluded from acceptance
and grant projection. The GUI disables Accept and names the blocker; the CLI
prints exact revoke/reject/delete resolution commands. Reject and local delete
remain available so a conflict cannot strand an unmanageable inbox row. An
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
That scope is visible next to the code and can be changed before accepting or
while the service is online. Windows firewall setup is attempted automatically;
if a normal rule fails, the app asks Windows for elevated firewall permission
through UAC.

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
