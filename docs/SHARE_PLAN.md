# Peer file sharing — implementation plan (#21 → build)

Goal: device-to-device file sharing where **our server only routes discovery**
(rendezvous/signaling); **file bytes go directly peer-to-peer, end-to-end
encrypted**. Two modes:

1. **Direct pair via code** — two devices enter the same short code; the server
   introduces them; they connect directly and transfer.
2. **File rooms** — a room has a code; any device with the code joins; files can
   be shared to **all** connected members. The app gets a new **Devices/Rooms
   view** alongside the file views.

A **standalone server binary** ships in the release for **Linux and Windows**.

---

## Components

```
share-server/                 # NEW standalone crate (headless, no GUI deps)
  src/main.rs                 # threaded TCP signaling + rendezvous (pair + rooms)
native/src/share/             # client module in the app
  mod.rs                      # public API used by app.rs (state machine, threads)
  proto.rs                    # wire types (shared shape, duplicated client/server)
  crypto.rs                   # code -> PSK (HKDF), Noise NNpsk0 channel (snow)
  transport.rs                # candidate dialing/listening, framed encrypted I/O
  xfer.rs                     # file offer/accept/stream, save to quarantine
native/src/app.rs             # Devices/Rooms view + settings (server address)
```

Why a separate server crate: it runs on the user's Linux box headless; it must
not pull in egui/wgpu/etc. Pure deps only (`serde`, `serde_json`; `tokio`-free —
a simple thread-per-connection `std::net` server scales fine here). Cross-compiles
to `x86_64-pc-windows-gnu` like the app, plus a native Linux build; both staged in
`release-native/share-server/`.

## Signaling protocol (newline-delimited JSON over TCP)

Client → server `Hello`:
```json
{"t":"hello","mode":"pair|room","code":"K7P2QX9F","device":"Laptop",
 "listen_port":51737,"lan":["192.168.1.5"],"pubkey":"<base64 x25519>"}
```
- Server observes the client's **public IP** from the socket and forms
  `candidates = lan.map(:listen_port) + [public_ip:listen_port]`.
- **pair**: server holds the first `Hello` per code; when the second arrives it
  sends each peer a `Peer{device,candidates,pubkey}` and the rendezvous is done.
- **room**: server keeps `code -> [member]`; on join it sends the newcomer a
  `Roster{members}` and tells existing members `Joined{member}`; on disconnect,
  `Left{device}`. The signaling socket stays open for roster updates.

Server → client: `Peer`, `Roster`, `Joined`, `Left`, `Error`. The server **never
sees file data** — only these control messages.

## Direct transport + crypto

- Each device opens a TCP **listener** on `listen_port` and advertises its
  candidates. To connect, the other device **dials all candidates in parallel**
  (LAN IPs first, then the server-observed public IP); first to connect wins.
  - LAN ↔ LAN: trivially direct. WAN: works when one side is reachable
    (port-forward / full-cone NAT via the public candidate) or via best-effort
    simultaneous-open.
  - **Honest caveat:** with **no relay** (server is router-only by design),
    **symmetric-NAT ↔ symmetric-NAT** pairs may fail — we show "couldn't connect
    directly" rather than relaying. (A future opt-in relay/TURN could be added.)
- **E2E encryption:** Noise **`NNpsk0`** (`snow` crate — pure Rust: X25519 +
  ChaChaPoly + BLAKE2s, no aws-lc) with **PSK = HKDF-SHA256("smart-explorer-share"
  ‖ code)**. Same code → same PSK → mutually authenticated encrypted channel; the
  server (and any eavesdropper) can't read content.
  - Code entropy: generate **8+ char Crockford-base32** (~40 bits) random codes so
    the low-entropy-PSK offline attack is impractical for ephemeral codes. (Future
    hardening: SPAKE2 PAKE to make even 6-digit codes safe.)

## File transfer (over the encrypted channel)

`Offer{name,size}` → receiver prompts **Accept/Reject** → on accept, sender
streams length-framed encrypted chunks → receiver writes to a **quarantine
folder** (`%USERPROFILE%/SmartExplorer-Empfangen`), sets **Mark-of-the-Web**
(Zone.Identifier) on Windows, never auto-executes. Room send = offer to every
member in turn.

## Safety (from the eval; enforced here)

- Receiving is **opt-in + per-transfer accept**; no silent writes.
- E2E encryption; **server only routes**, logs no content.
- Received files quarantined + MOTW + never executed.
- Rooms/pairings are code-gated; codes are random + ephemeral; **leave/disband**
  drops membership. Rate-limit + size caps on the server.
- Device identity = its X25519 pubkey shown as a short fingerprint for the user
  to eyeball.

## UI — new "Geräte / Räume" view

A toggle in the toolbar (and a sidebar section) opens a panel:
- **Pair:** "Code anzeigen" (generates code, registers, waits) / "Mit Code
  verbinden".
- **Räume:** "Raum erstellen" / "Raum beitreten" (code) → live member list with
  fingerprints; select files in the normal view → **An Raum/Gerät senden**.
- **Eingehend:** accept/reject prompts + transfer progress; "Empfangen" opens the
  quarantine folder.
- Settings → **SHARE**: rendezvous server address (default = the maintainer's
  host), device name.

## Build / release

- `share-server` built for Linux (native) + Windows (gnu) by `publish-feed.sh`,
  staged in `release-native/share-server/` and attached by CI.
- Client `share` module gated behind the view; dormant until a server address is
  set, so it can't affect existing users.

## Verification limits

Live P2P/NAT/crypto handshake can't be exercised in the headless build env. Every
slice compiles for host + `x86_64-pc-windows-gnu`, the server also for native
Linux, and the pure logic (code gen/parse, HKDF determinism, JSON framing, room
bookkeeping) is unit-tested. The networked path needs a real two-machine test.

## Slices (each compiles + ships)
1. **share-server** crate (pair + rooms signaling) + tests + in release. ← first
2. Client `proto`+`crypto`+`transport` (connect, Noise, candidate dial) + tests.
3. Client `xfer` (offer/accept/stream to quarantine) + the **Devices/Rooms view**
   for direct pair.
4. Rooms in the UI (create/join, roster, send-to-all).
