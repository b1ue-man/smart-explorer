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
