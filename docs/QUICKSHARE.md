# Quick Share (Android Nearby Share) interop — plan & status

Goal: send/receive files to/from Android (and Windows) **Quick Share** devices on
the same Wi-Fi. Reference implementation: `rquickshare` (Rust) and `NearDrop`.

## Shipped now (0.5.28)

**LAN discovery** (`quickshare.rs`, pure-Rust `mdns-sd`): browses/advertises the
Quick Share mDNS service `_FC9F5ED42C8A._tcp`, so nearby Quick Share endpoints
appear in 📡 Teilen → "Quick Share (LAN)". Runs only while the Teilen view is
open. This is the discovery foundation; it does not yet transfer bytes.

## Remaining (the transfer layer — needs real-device iteration)

Quick Share's offline/Wi-Fi transfer is **Nearby Connections**:

1. **Discovery** ✅ mDNS (done) + (optionally) a **BLE advertisement** to wake
   Android's "Everyone" visibility (WinRT `Windows.Devices.Bluetooth.Advertisement`).
2. **Transport**: TCP to the advertised endpoint (host:port from mDNS).
3. **UKEY2 handshake** (`securemessage` + `ukey2` protobufs): X25519 ECDH →
   HKDF → an authenticated AES-256-CBC + HMAC-SHA256 session. (RustCrypto has all
   primitives; the message flow must match Google's exactly.)
4. **Nearby Connections frames**: length-prefixed **protobuf** `OfflineFrame`s
   (CONNECTION_REQUEST/RESPONSE, PAYLOAD_TRANSFER with chunked file bytes +
   KeepAlive), wrapped in the UKEY2 session.
5. **Payload**: introduction (file metadata) → accept → chunked `PAYLOAD_TRANSFER`.

### Implementation approach
- Add `prost` + the `.proto` files (from rquickshare / the Nearby Connections
  spec): `securemessage.proto`, `ukey.proto`, `offline_wire_formats.proto`,
  `wire_format.proto`; codegen in `build.rs`.
- Implement the UKEY2 client+server flow + the OfflineFrame state machine over
  the `share`-style framed TCP, reusing the quarantine-save + accept-prompt UX.
- BLE advertise via WinRT (Windows) to be discoverable by Android in "Everyone".

### Why it's not done blind
The protobuf + UKEY2 flow must byte-match Google's implementation; it cannot be
verified without a real Android device on the same network. It's a sizable,
iterate-against-hardware effort. **The own paired share (📡 Teilen, E2E,
server-routed) already provides working cross-device transfer today**; Quick
Share interop is the "talk to stock Android Quick Share without our app" bonus.

## AirDrop
Smart Explorer still cannot implement native AirDrop from Windows: AirDrop's
direct radio path uses Apple's proprietary **AWDL**, and Windows exposes no
AWDL API to third-party apps. What changed externally is Android/Google Quick
Share itself: compatible Android devices can now exchange files with Apple
AirDrop devices when the Apple side is set to "Everyone for 10 minutes".

That does **not** make this Smart Explorer prototype AirDrop-compatible. Our
code only does LAN Quick Share discovery today; it does not inherit Google's
closed Android-side AirDrop bridge. For iPhone/iPad/macOS interop, users should
use the platform Quick Share/AirDrop path where supported; Smart Explorer's own
remaining work is still the UKEY2 + OfflineFrame Quick Share transfer layer.

Refs checked 2026-06-28: <https://blog.google/products-and-platforms/platforms/android/quick-share-airdrop/>,
<https://support.google.com/pixelphone/answer/9286773>.
