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

Multiple app endpoints can be separated with commas or semicolons:

```text
wss://share.example.com/se-share, share.example.com:51820
```

## HTTPS / 443 Deployment

Run the server locally and terminate TLS in a reverse proxy:

```text
se-share-server 127.0.0.1:51820
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
location /se-share {
    proxy_pass http://127.0.0.1:51820;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
}

location / {
    proxy_pass http://127.0.0.1:51821;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
}
```

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
| Signaling route result (`relay.*`) | `forwarded`, `target_offline`, or unconfirmed | What the rendezvous server did with the latest access envelope. This is not peer receipt or Iroh data-path status. |
| Decision | `pending`, `accepted`, `rejected`, `revoked`, `failed`, `expired` | Latest verified access decision and its revision. |
| Decision delivery | `not_started`, `queued`, `sent`, `server_queued`, `delivered`, `received`, `failed`, `expired` | Progress of the signed decision envelope. On the deciding endpoint, `received` requires the peer's signed decision receipt. |
| Authorization | `active` or `inactive` | Whether the verified decision is currently projected into a local contact/grant. |
| Connectivity | offline/waiting/connecting/direct/relay/error state | Current data-path reachability. It is independent of authorization and signaling delivery. |

Consequently, `queued` means only "durably stored on this endpoint". A signaling
route ACK of `forwarded` means only that the server enqueued the envelope to a
currently connected compatible client writer. It does not mean that the peer
read, verified, or persisted it. `target_offline` is likewise a retryable
signaling observation, not a rejected request. These route ACKs are distinct
from connectivity such as `connected_relay`, which describes an authenticated
Iroh data session using the encrypted transport relay.

The rendezvous server does not store a direct-access inbox. Durable outbox,
inbox, retry counters, signed envelopes, receipts, decisions, and grants live on
the endpoints. Replaying the same request keeps the same request ID and is
idempotent. Clients negotiate `tracked_direct_v1`; old fallback messages remain
visible as legacy/unconfirmed because they cannot prove all lifecycle states.

## Manage Requests and Grants

The GUI's **Teilen** view shows three durable sections:

- **Eingehende Anfragen**: request ID, requester identity, fingerprint,
  transport/receipt state, decision, authorization, and actions to accept or
  reject after explicitly confirming the displayed fingerprint.
- **Ausgehende Anfragen**: the same lifecycle state from the requester side and
  a retry action that reuses the request ID.
- **Autorisierte Geraete**: active grants and connectivity, with signed revoke
  for tracked grants. A legacy grant can only be disabled locally and is marked
  as such.

The terminal exposes the same ledger:

```text
se share request list [--json]
se share request show <request-id> [--json]
se share request accept <request-id> --fingerprint <fingerprint> [--message <text>] [--json]
se share request reject <request-id> --fingerprint <fingerprint> [--message <text>] [--json]
se share request retry <request-id> [--json]
se share grants list [--json]
se share grants revoke <request-id> --fingerprint <fingerprint> [--message <text>] [--json]
se share status [--json]
```

`request list/show` and `status` report delivery, relay result, peer receipts,
decision and revision, retry attempts/errors, authorization, and connectivity
separately. Fingerprint arguments must exactly match the identity shown by
`list`/`show`; this prevents an acceptance or revocation from silently applying
to a different key. Profile mutations use bounded compare-and-swap retries, so
normal concurrent GUI/worker updates are rebased instead of overwriting each
other.

The accepted direct code authorizes the owner's `Standard Direkt` export scope.
That scope is visible next to the code and can be changed before accepting or
while the service is online. Windows firewall setup is attempted automatically;
if a normal rule fails, the app asks Windows for elevated firewall permission
through UAC.

## Lifecycle Regression Guard

CI runs native Windows library and standalone-`se` tests on a Windows runner.
The cross-platform build also runs `native/test-share-lifecycle-e2e.sh` with two
isolated Linux client profiles and a real `se-share-server`. That scenario
checks an initially offline target, same-ID retry, pending-inbox survival across
daemon restart, offline acceptance delivery after requester restart, both
signed receipts, an operational file listing after acceptance, signed revoke,
and denied file access after revocation.
