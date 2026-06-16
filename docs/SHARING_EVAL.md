# Device-to-device sharing — feasibility evaluation (#21)

Question: can Smart Explorer share content using the **real** Quick Share and
AirDrop protocols, and/or via **direct device pairing** (with the maintainer's
server available for "own DNS routing" / rendezvous)? What dangers must be
mitigated? This is an evaluation, not an implementation.

TL;DR:
- **AirDrop: not feasible on Windows.** ❌ (hard hardware/driver blocker)
- **Quick Share: feasible**, same-Wi-Fi, with real Android interop. ✅ (a Rust
  implementation already exists to reference)
- **Own paired E2E share via your server: feasible and the best fit** — it's the
  only option that works *across networks* (WAN), reuses our transfer code, and
  we fully control it. ✅ (recommended)

---

## 1. AirDrop (real protocol) — ❌ not feasible on Windows

AirDrop runs **only** over **AWDL (Apple Wireless Direct Link)**, a proprietary
Apple Wi-Fi protocol. The open reimplementation (`owl` + `OpenDrop`, SEEMOO Lab /
Open Wireless Link) needs the Wi-Fi adapter in **active monitor mode with raw
frame injection** (libpcap). That is available on some Linux/macOS setups but
**Windows Wi-Fi drivers do not expose monitor mode / injection**, so AWDL — and
therefore AirDrop — cannot run on a normal Windows machine. Apple ships custom
wireless hardware to make AirDrop seamless; there is no Windows equivalent.

Verdict: a Windows app cannot speak real AirDrop. We would only ever interop with
Apple devices by routing through a Mac/Linux box running OWL — not viable for a
shipping Windows tool. **Recommend dropping AirDrop.**

### Update — the 2025 "Quick Share ↔ AirDrop" interop (researched)

Late 2025: Google added AirDrop interop to **Quick Share** (Pixel 10 first, then
Samsung/OPPO/OnePlus/Xiaomi/…). Android can now send to / receive from Apple's
**AirDrop "Everyone for 10 minutes"** mode, direct P2P, no server. **This does
not change our verdict**, because:

- It's **Google's own closed reverse-engineering of AirDrop, built into Android's
  Quick Share client** (no Apple partnership, spec still unpublished). Nothing was
  released that a third party could build against — "Windows developers can't
  build compatible implementations even if they wanted to."
- The transport is still **AWDL** (proprietary Apple peer-to-peer Wi-Fi). Android
  reaches it via OS-level radio support; **Windows exposes no AWDL / Wi-Fi-Aware
  API to apps**, and the owl/OpenDrop monitor-mode route isn't viable on Windows.
- The realistic "AirDrop on Windows" answer is **Google's official Quick Share
  *for Windows* app** (which is gaining AirDrop bridging) — i.e. a separate app,
  not something we implement. Our own Quick Share implementation (below) talks to
  **Android**, and would **not** inherit the AirDrop bridge.

So: still ❌ for us to implement. If "reach iPhones from Windows" matters, the
pragmatic route is to point users at Google's Quick Share for Windows, not build it.

## 2. Quick Share / Nearby Share (real protocol) — ✅ feasible (same Wi-Fi)

Google's Quick Share (ex-"Nearby Share") is reverse-engineered and reimplemented
in **Rust** today: **`rquickshare`** (Martichou; Linux/macOS) and the
**`rquickshare-x`** fork (adds **Windows**). It interoperates with real Android
devices (Android 6+). How it works, and what we'd need:

- **Discovery:** **mDNS** on the LAN, plus a **BLE advertisement** to nudge
  Android into making its mDNS service visible (Android doesn't broadcast it
  continuously even in "Everyone" mode).
- **Auth/crypto:** a **UKEY2** key exchange → authenticated AES channel.
- **Payloads:** length-prefixed **protobuf** frames; the bulk transfer runs over
  **TCP on the same Wi-Fi network** (Google also uses Wi-Fi Direct/hotspot for
  cross-network-less cases).
- **What we'd add on Windows:** mDNS (pure Rust crates exist), BLE advertising
  via **WinRT** Bluetooth APIs, and the UKEY2/protobuf state machine (reuse
  `rquickshare`'s `core` crate if its license permits — **check the license
  before vendoring**).

Constraints: **both devices must be on the same Wi-Fi** and the network must
allow mDNS (many corporate/guest APs block multicast). No cross-internet
transfer. Effort: meaningful but bounded, with a working Rust reference.

Verdict: doable as an **interop bonus** for "send to my Android phone on the same
network." Not a WAN solution.

## 3. Own paired device-to-device share (recommended) — ✅ feasible

This is the best fit for us: an **own protocol** we control, reusing the existing
`vfs::Backend` + transfer code, with your **server as a rendezvous/relay** so it
works **across networks** (the thing AirDrop and stock Quick Share can't do).

**Design constraint from the maintainer:** the server is a **router/rendezvous
ONLY — never a file-transfer gate.** All file bytes go **directly device-to-device
(P2P)**. The server exists purely because, without a fixed point, two devices on
different networks can't *find* each other; it must never see or forward content.

Architecture options:
- **LAN fast path:** mDNS discovery + a direct TLS/Noise TCP connection
  (no server involved at all when both are on the same network).
- **WAN path (your server = signaling + STUN only):** the server is a
  **rendezvous/signaling** point — your DNS routing maps a stable device name →
  its current public endpoint — plus a **STUN**-style reflector so each side
  learns its own NAT-mapped address. The two devices then **hole-punch a direct
  connection** (ICE / UDP hole punching; WebRTC data channels do exactly this and
  give DTLS E2E for free). The server brokers the handshake and **drops out** —
  payload never traverses it.
  - *Caveat (honest):* with **no TURN relay**, **symmetric-NAT ↔ symmetric-NAT**
    pairs (some carrier-grade/corporate networks) can't be hole-punched and the
    transfer simply won't establish. That's the price of "server is router only."
    We surface a clear "couldn't establish a direct connection" message rather
    than silently relaying. (A relay could be an explicit, opt-in later fallback.)
- **Pairing:** short-code **PAKE** (SPAKE2) or a **QR code** carrying a
  pre-shared key; after first pairing, **pin each device's public key** (TOFU) so
  later transfers are silent-but-authenticated. The server only ever holds public
  routing info, never keys or content.
- **Reuse:** received/sent bytes flow through the same streaming code we already
  use for sync/upload.

### Bonus idea (maintainer's) — peer runs Smart Explorer as a "remote agent"

Instead of treating the far device as a dumb byte store, **run Smart Explorer on
both ends** and let the remote instance act as an **agent**: it executes the
filesystem operations *locally* (directory scan, deep filter, fuzzy index/search)
and sends back only the **results** over the P2P channel. This is a natural new
`vfs::Backend` ("peer agent") whose `list_dir`/scan/search calls are RPCs to the
peer rather than raw file reads — so browsing/searching a remote machine is as
fast as local (the heavy walk happens on the machine that owns the disk; only
compact result rows cross the wire), and actual file bytes still transfer
directly only when opened/copied. Big UX win and it fits our architecture cleanly;
worth designing in from the start of the `share`/`peer` module.

### Dangers to mitigate (and how)
| Risk | Mitigation |
|---|---|
| **Unsolicited files / "AirDrop-Everyone" spam** | Require explicit **pairing** first; **per-transfer accept** prompt; discoverability **opt-in and time-boxed**, off by default. |
| **MITM during pairing** | **PAKE** from a short code, or a **short-auth-string** the two users compare; pin keys afterwards. |
| **Eavesdropping / your server reading content** | **End-to-end encryption** (Noise or DTLS via WebRTC); the relay only ever sees **ciphertext**; never log payloads. |
| **Malicious received files** | Save to a **quarantine folder**; set **Mark-of-the-Web** (`Zone.Identifier`); **never auto-execute**; warn on executables/scripts; optional AV scan; preserve, don't run. |
| **Replay / duplication** | Per-session keys + **nonces**; idempotent transfer IDs. |
| **Presence/privacy leakage** | Rotate device IDs; don't advertise stable identifiers; user-named, user-approved devices. |
| **DoS / resource abuse** | Require accept before receiving; **rate-limit**; cap concurrent transfers/size; relay **quotas** + session expiry. |
| **Relay/server compromise** | Authenticate clients to the rendezvous; E2E means a breached relay leaks only ciphertext + metadata; minimise stored metadata. |
| **Cross-device trust revocation** | Let the user **unpair**/revoke a device (drop its pinned key). |

Verdict: **recommended primary** — most value (works anywhere via your server),
full control, reuses our code, and the danger set is well-understood and
mitigable with standard crypto (PAKE + Noise/DTLS).

---

## Recommendation & rough effort
1. **Drop AirDrop** (impossible on Windows).
2. **Build the own paired share** (LAN direct + WAN via your server as **router
   only**, content always direct P2P). Phase it: (a) LAN mDNS + Noise + accept-
   prompt + quarantine; (b) your server as **signaling + STUN** so WAN peers
   hole-punch a **direct** connection (no relay); (c) PAKE/QR pairing + key
   pinning + unpair; (d) the **peer-agent `Backend`** so the far Smart Explorer
   runs scans/filters/searches locally and streams back results. Medium-large
   effort; isolated as a new `share`/`peer` module + a small signaling server.
3. **Optionally add Quick Share interop** later for same-network Android sends,
   referencing `rquickshare` (license-permitting).

Decision needed from you: pursue **(2) own paired share** first (recommended), and
do you want **(3) Quick Share interop** too? I can then write an implementation
plan like `docs/CLOUD_OAUTH_PLAN.md` and start the `share` module.

---

### Sources
- AWDL/OpenDrop, Windows monitor-mode blocker: <https://owlink.org/>,
  <https://github.com/seemoo-lab/opendrop>, <https://github.com/seemoo-lab/owl>,
  <https://bakedbean.org.uk/posts/2021-05-airdrop-anywhere-part-1/>
- Quick Share in Rust: <https://github.com/Martichou/rquickshare>,
  <https://github.com/oop7/rquickshare-x>
