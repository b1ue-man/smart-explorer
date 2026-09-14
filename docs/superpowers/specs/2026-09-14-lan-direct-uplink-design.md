# LAN Direct Presence and Automatic Uplink Sharing — Design

Status: approved design (decisions delegated by the user on 2026-09-14), part of
the batch that also repairs connection deletion and Google Drive duplicate
names (see the plan in `docs/superpowers/plans/2026-09-14-connection-cleanup-lan-batch.md`).

## Goal

Two or more Smart Explorer devices that are wired together **without a
router** (direct cable or a dumb switch, no DHCP server, no default gateway)
must find each other and open their existing Direct relation with **zero
clicks**, without the Share server. When exactly one side of such a link has
internet access on another interface, that side becomes the router and shares
its internet with the paired devices — automatically after a one-time
authorization.

Decisions taken (delegated):

| Question | Decision |
|---|---|
| Activation of uplink sharing | One-time authorization (persistent setting + one UAC/polkit consent), afterwards fully automatic. |
| Platforms | Windows (ICS via `INetSharingManager`) and Linux (NetworkManager `ipv4.method=shared`). Anything else reports *not available* with the exact reason. |
| LAN presence scope | Always on (mDNS on every interface). Uplink sharing is restricted to router-less links. |
| Sync jobs on a deleted connection | Kept, shown as orphaned (Part 1). |

Global rule (user requirement): **never assume a facility exists.** Every OS
facility (mDNS socket, interface enumeration, IPv4 link-local, NetworkManager,
polkit/pkexec, dnsmasq, ICS COM, `SharedAccess` service, Task Scheduler) is
probed; a missing facility degrades that feature only, is reported in the UI,
CLI and daemon log with the concrete reason, and never blocks the rest of
Share. Where a facility is available the feature must work.

## Stage 1 — LAN presence (offline Direct)

### What exists

A Direct relation is a `DirectContact` (outgoing, with relation secret and
pinned `expected_node_id`/`remote_public_key`) plus a `DirectGrant` on the
receiving side. Dialing needs a `PeerPresence` (`node_id`, `candidates`,
`relay_url`) which today only the Share server delivers (signed with the
relation secret, verified in `signal_auth.rs`). Transport is Iroh QUIC with the
relation secret proven inside the session handshake (`session.rs`), so
presence is *routing evidence only*; identity is proven by the Iroh TLS node
key and the HMAC session proof.

### Design

1. **Announcement** (`share/os/shared/lan_presence.rs`, `mdns-sd`, already a
   dependency): the worker registers `_se-share._udp.local.` with instance name
   `se-<id8>`, host `<sanitized hostname>.local.`, `enable_addr_auto()`, port =
   Iroh IPv4 bound port, TXT `v=1`, `id=<hex16>`, `p4=<port>`, `p6=<port>`,
   `up=0|1`. `id` = first 16 hex chars of `SHA-256("se-lan-presence-v1" ||
   node_id)`; the raw Iroh key is not broadcast. `up` is the device's own
   uplink verdict (Stage 2 facts) and is only advisory.
2. **Browse**: the same daemon browses the service. Every `ServiceResolved`
   yields `LanSighting { id, addrs, p4, p6, uplink, seen_at }`; `ServiceRemoved`
   yields a loss. Sightings are matched in `share/core/lan_presence_match.rs`
   (pure): a contact matches when
   `hash(contact.expected_node_id) == id` (or `remote_public_key` when the node
   pin is empty) and `access_state == Accepted` and `remote_device_id` is set.
   Unknown ids are ignored (no logging of foreign devices beyond a counter).
3. **Candidates**: IPv4 addresses become `ip:p4`. IPv6 link-local addresses
   need a scope; they are emitted as `[fe80::x%<ifindex>]:p6` for every local
   interface that is up and non-loopback. `session.rs::endpoint_addr` learns to
   parse the `%<ifindex>` form into `SocketAddrV6` with that scope id. Global
   IPv6 addresses are emitted plainly.
4. **State**: new fields on `DirectContact` (all `#[serde(default)]`):
   `lan_candidates: Vec<String>`, `lan_seen_at: Option<i64>`,
   `lan_uplink: Option<bool>`. New `ShareEvent::LanPeerSeen { contact_id,
   candidates, uplink }` and `ShareEvent::LanPeerLost { contact_id }`. The daemon
   (`ipc_host_events.rs`) persists them like server presence; `DirectOffline`
   from the server does not clear LAN evidence, and LAN evidence expires on its
   own after `LAN_PRESENCE_TTL_SECS = 150` without a refresh (mDNS re-announces
   every 60 s).
5. **Dialing** (`service.rs::endpoint_for_target`): with a current server
   presence, LAN candidates are prepended to `presence.candidates`. Without one,
   but with fresh LAN evidence, a presence is synthesized from the contact pins
   (`device_id`, `public_key`, `fingerprint`, `node_id`), `relay_url = ""`,
   `nonce = "lan"`, `proof = ""`, `expires_at = lan_seen_at + TTL`. No code path
   verifies `proof` after admission (verification lives at the signaling
   boundary only), and the TLS node pin plus session proof still authenticate.
6. **Service lifetime**: `share_service_requested` also returns true when the
   server is empty but LAN presence is enabled (default) and Direct contacts
   exist, so Share works with no server configured. `transport_options::load("")`
   already yields relay-disabled options.
7. **Auto-open**: the GUI treats `LanPeerSeen` like `DirectAvailable` for the
   existing `auto_open` contact flag.
8. **Not available** cases: mDNS daemon start failure (socket/firewall),
   no non-loopback interface, Iroh not bound. Each is reported as
   `LanStatus { presence: Unavailable(reason) }` in the Share view "LAN" section
   and `se share lan`.

## Stage 2 — Automatic uplink sharing

### Facts (`net/core/link_facts.rs`, pure; adapters under `net/os/*`)

`InterfaceFacts { name, index, up, loopback, addrs: Vec<IpAddr>, has_gateway,
dhcp_lease: Option<bool> }` gathered per tick:

* Windows: `GetAdaptersAddresses` (IP Helper): `OperStatus`, unicast
  addresses, `FirstGatewayAddress`, `Dhcpv4Enabled`/`Dhcpv4Server`, `IfIndex`.
* Linux: `if-addrs` for addresses, `/sys/class/net/<if>/operstate` and
  `/carrier`, `/proc/net/route` (flag `RTF_GATEWAY` with destination 0) and
  `/proc/net/ipv6_route` for gateways; DHCP lease knowledge from NetworkManager
  (`Device.Dhcp4Config != "/"`) when NM is available, otherwise `None`.

Classification (`LinkClass`):

* `RouterLess` — `up && !loopback && !has_gateway && dhcp_lease != Some(true)`
  and every IPv4 address is link-local (`169.254/16`) or absent, or the only
  addresses are IPv6 link-local. An interface carrying our own ICS/NM-shared
  gateway address (recorded in state) stays `RouterLess`.
* `Uplink` — `up && has_gateway` and internet reachability: Windows
  `NetworkInformation::GetInternetConnectionProfile()` reports a profile whose
  adapter matches, Linux NM `Connectivity == FULL (4)`; when neither is
  available, a gateway alone counts as uplink. An interface where a paired LAN
  peer is present is never an uplink (prevents the "my only gateway is the SE
  router" loop on the receiving side).
* `Routed` — everything else.

### Policy (`share/core/lan_uplink_policy.rs`, pure state machine)

Inputs each tick: settings (`uplink_sharing_enabled`), classified interfaces,
LAN sightings of *paired* contacts (interface index, `uplink` advisory), current
`UplinkState`. Output: `Decision::{Keep, Start { private_if, public_if },
Stop { reason }, Idle(reason) }`.

Rules:

1. Feature disabled or platform adapter unavailable → `Idle(reason)`.
2. Candidate link = a `RouterLess` interface with at least one paired peer
   seen within `PEER_PRESENT_SECS = 60` whose advisory `uplink == false`.
3. Own uplink = exactly the set of `Uplink` interfaces excluding candidate
   links; empty → `Idle("kein eigener Internetzugang")`. If a peer reports
   `uplink == true` → `Idle("Peer hat eigenen Internetzugang")` (both have
   internet: nobody shares; the tie-break for two uplinks with `up=1` on both is
   the lexicographically smaller hashed id shares — reported, not silently).
4. Start only after the candidate has been stable for `START_DEBOUNCE_SECS = 5`.
5. Stop when the peer is absent for `STOP_GRACE_SECS = 90`, the uplink is gone
   for `UPLINK_LOSS_SECS = 15`, the setting is disabled, or the daemon shuts
   down. Reconcile at daemon start: state file says *sharing* but no peer →
   stop.

### Apply

* Windows (`net/os/windows/ics.rs`): COM `NetSharingManager`
  (`windows` crate feature `Win32_NetworkManagement_WindowsFirewall`):
  `SharingInstalled()`, `EnumEveryConnection` → match `INetConnectionProps.Guid`
  with the adapter GUID (`GetAdaptersAddresses.AdapterName`), then
  `EnableSharing(ICSSHARINGTYPE_PUBLIC)` on the uplink and
  `ICSSHARINGTYPE_PRIVATE` on the LAN interface; `DisableSharing` on both to
  stop. Needs administrator rights, so it runs in the **helper**.
* Windows helper (`net/os/windows/uplink_helper.rs`, internal mode
  `se.exe --lan-uplink-helper`): a Scheduled Task `Smart Explorer LAN-Uplink`
  (RunLevel Highest, current user, on demand) is created once via the existing
  elevated PowerShell pattern (`share/os/windows/system.rs`). The daemon writes
  `lan_uplink/request.json` (`{op: enable|disable, public_guid, private_guid,
  issued_at, nonce}`), runs `schtasks /Run /TN ...` unelevated, and waits for
  `lan_uplink/response.json` with the same nonce. The helper re-validates:
  request younger than 60 s, both GUIDs exist, the private adapter is
  router-less per its own facts, the setting file says enabled. Availability
  probes: `SharingInstalled`, `SharedAccess` service not `Disabled`
  (`sc qc SharedAccess`; the one-time setup sets it to `demand` when disabled),
  task exists. Any failure → `Unavailable(reason)`.
* Linux (`net/os/linux_os/nm_shared.rs`, zbus system bus,
  `org.freedesktop.NetworkManager`): `GetDeviceByIpIface`, device `State`,
  `Ip4Config.Gateway`, `Dhcp4Config`, `Connectivity`; to start:
  `Settings.AddConnection` (or reuse) of a profile
  `Smart Explorer LAN-Uplink (<iface>)` with `connection.interface-name`,
  `connection.autoconnect=false`, `ipv4.method=shared`, `ipv6.method=ignore`,
  then `ActivateConnection`; to stop: `DeactivateConnection` and delete the
  profile so NM autoconnects the user's normal profile again. Availability:
  NM name owned on the system bus; `dnsmasq` on `PATH` (NM needs it for shared
  mode); polkit consent. One-time setup installs
  `/etc/polkit-1/rules.d/49-smart-explorer-lan-uplink.rules` via `pkexec`
  allowing `org.freedesktop.NetworkManager.settings.modify.system` and
  `org.freedesktop.NetworkManager.network-control` for the current user; when
  `pkexec` is missing the feature still tries (desktop polkit agents may prompt)
  and reports `AccessDenied` as *not available* with the manual rule text.

### Settings, state, UI, CLI

* `lan_settings.json` (app data): `{ "presence_enabled": true,
  "uplink_sharing_enabled": false, "uplink_setup_done": false }`.
* `lan_uplink_state.json`: `{ "sharing": {private_if, public_if, since}?,
  "last_error" }` for reconcile.
* Share view gets a fifth tab **LAN**: presence status, seen paired peers with
  interface and address, link classification per interface, uplink status,
  the `Internet automatisch an gekoppelte Geräte teilen` toggle (turning it on
  runs the one-time setup and reports the outcome), current sharing state with
  reason, `Jetzt beenden`.
* CLI: `se share lan [--json]` (status), `se share lan uplink enable|disable`.
* Daemon log lines for every transition.

### Security notes

* Sharing is enabled only for interfaces where a *paired* contact announces
  itself; the announcement is unauthenticated, so a stranger who spoofs a
  paired id could trigger ICS on a router-less link he is physically plugged
  into. The helper's own facts check limits this to router-less links, and the
  setting is off by default. Documented in `docs/GOTCHAS.md`.
* The Windows task/helper is bound to the installed `se.exe` path and validates
  the request file; it never enables sharing on a routed interface.
* The hashed id does not reveal the Iroh key but is a stable LAN identifier,
  comparable to the mDNS hostname.

## Out of scope

Wi-Fi hotspot, IPv6 prefix delegation on the shared link, per-device NAT
restrictions, macOS.
