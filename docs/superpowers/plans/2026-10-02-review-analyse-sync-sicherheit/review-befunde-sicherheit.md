# Review-Befunde: Sicherheit der Share-Übertragungen (Direct und Räume)

Stand: 2026-10-02. Quelle: Review-Workflow (Finder je Dimension, unabhängige Gegenprüfung, Vollständigkeits-Kritiker). Fehlende Dimensionen: keine.

| ID | Schwere | Urteil | Dimension | Titel |
|---|---|---|---|---|
| S01 | medium | confirmed | transport-encryption | Share signaling is plaintext by default: bare host:port, tcp://, ws:// and http:// are accepted silently and se-share-server cannot speak TLS |
| S02 | low | confirmed | transport-encryption | Multi-endpoint server lists silently downgrade from wss:// to plaintext and mix TLS and plaintext relays |
| S03 | medium | confirmed | transport-encryption | Relay URLs default to plaintext http:// and the bundled relay has no TLS |
| S04 | high | confirmed | transport-encryption | Default export is the whole home (Android: whole shared storage), read-write, for every Direct code holder, every reciprocal pair and every room member |
| S05 | medium | confirmed | transport-encryption | Blocking a room member is not a revocation: the room secret never rotates and new identities are auto-admitted |
| S06 | medium | confirmed | transport-encryption | Presence MAC excludes device_name and fingerprint; receivers store and display them |
| S07 | medium | confirmed | transport-encryption | Unauthenticated peers can exhaust the 64 pre-authentication handshake slots and lock out all incoming Share sessions |
| S08 | medium | confirmed | transport-encryption | Signaling registrations are unauthenticated: lookup ids and room ids act as bearer tokens for presence data and entry replacement |
| S09 | medium | confirmed | transport-encryption | LAN uplink sharing (opt-in) is started by unauthenticated mDNS announcements |
| S10 | medium | confirmed | transport-encryption | Elevated uplink helper task runs a user-writable executable (silent UAC bypass) and executes a user-writable setup script |
| S11 | low | confirmed | transport-encryption | Linux polkit rule grants permanent passwordless system-wide NetworkManager control |
| S12 | low | confirmed | transport-encryption | FS handshake reveals the exact authorization failure to unauthenticated peers |
| S13 | low | confirmed | transport-encryption | Signaling Hello sends all local IPv4 addresses and device name that the server never uses |
| S14 | low | confirmed | transport-encryption | LAN presence is on by default on every network with a stable trackable id and hostname; unauthenticated sightings are unbounded and overwrite routes |
| S15 | low | confirmed | transport-encryption | Firewall rule opens the whole executable on all network profiles and is created automatically |
| S16 | low | confirmed | transport-encryption | Presence replay cache is flushed wholesale at 4096 entries |
| S17 | low | confirmed | transport-encryption | node_id is not bound to public_key; unpinned legacy contacts/members dial the node id from presence |
| S18 | low | confirmed | transport-encryption | ws:// and http:// endpoints derive the relay on the signaling port, so relay fallback silently fails |
| S19 | high | confirmed | pairing-identity-rooms | Removed or revoked Direct peers can get access back by using a new device_id, because the device-wide Direct code is accepted automatically and the removal record checks only the device_id |
| S20 | critical | confirmed | pairing-identity-rooms | PIN discovery lets strangers pair by default: the desktop GUI publishes with an empty PIN, there is no strength minimum and no lockout, offers stay open after a successful pairing, and every client of the server sees the offer list |
| S21 | critical | confirmed | pairing-identity-rooms | Pairing or joining a room exposes the whole home folder read/write/delete by default, and Direct pairing automatically grants the other side access |
| S22 | high | confirmed | pairing-identity-rooms | Room members cannot really be revoked: blocking is keyed on a device_id the member chooses, the room secret cannot be rotated, and members' identities are not authenticated |
| S23 | high | confirmed | pairing-identity-rooms | After a Direct code rotation or identity repair, every former peer is treated as explicitly ignored and cannot re-pair |
| S24 | medium | confirmed | pairing-identity-rooms | PIN pairing grants access before the other side commits; an aborted exchange shows 'failed' although the grant is already active |
| S25 | medium | confirmed | pairing-identity-rooms | Share-server registrations are unauthenticated: anyone can take over a device's lookup_id registration, push it out of a room roster, or receive messages addressed to its device_id |
| S26 | medium | confirmed | pairing-identity-rooms | Presence HMAC does not cover fingerprint or device_name, yet the receiver stores both and uses the fingerprint in authorization checks |
| S27 | medium | confirmed | pairing-identity-rooms | Signaling falls back to unencrypted transport without warning; a bare host:port uses plaintext TCP |
| S28 | medium | confirmed | pairing-identity-rooms | Anyone holding the Direct code can fill the 24-entry request ledger and the 24-entry legacy inbox with auto-accepted entries that are never pruned, blocking all new requests |
| S29 | low | confirmed | pairing-identity-rooms | Removing a Direct contact whose remote_device_id is not yet known leaves that device's active grant in place and records no removal |
| S30 | medium | confirmed | pairing-identity-rooms | Direct presences and legacy accept/reject messages are authenticated only by the target's device-wide Direct secret, so any other holder of that code can forge them to the target's other peers |
| S31 | low | confirmed | pairing-identity-rooms | Windows: the Share identity roams with the user profile (Roaming AppData plus Credential Manager with ENTERPRISE persistence), so several domain PCs can end up with the same identity |
| S32 | low | confirmed | pairing-identity-rooms | Clock skew between devices makes tracked envelopes fail with permanent errors until the receiver's clock catches up |
| S33 | low | partially_confirmed | pairing-identity-rooms | Secrets are stored unencrypted at rest on Linux and Android, and the Linux identity and profile files are readable by other local users |
| S34 | low | confirmed | pairing-identity-rooms | ShareIdentity and PeerEndpoint derive Debug while holding raw 32-byte secrets |
| S35 | low | confirmed | pairing-identity-rooms | The identity transaction lock waits indefinitely, with no timeout or diagnostic |
| S36 | high | confirmed | authorization-host-access | Share exports have no read-only mode; every accepted Direct/Room peer gets read-write, and the first-run default export is the whole home directory |
| S37 | medium | confirmed | authorization-host-access | No per-principal fairness on the 32-slot metadata/control pool: one authorized peer can stall browsing, stat, walk, snapshot, rename and delete for all peers |
| S38 | low | partially_confirmed | authorization-host-access | Peer-triggered storage analysis adds a canonicalize()+symlink_metadata() syscall per directory that the local scan does not, slowing remote analysis (matches the phone->PC slowness report) |
| S39 | low | confirmed | authorization-host-access | Dead/misleading 'block symlinks outside the share' checkbox: the flag is never read and confinement is unconditional |
| S40 | low | partially_confirmed | authorization-host-access | Single reads are not re-authorized per chunk; an un-shared file can keep streaming during the asynchronous connection teardown window |
| S41 | low | partially_confirmed | authorization-host-access | Established peer connections are uncapped; the handshake limiter only bounds in-flight handshakes, letting one peer accumulate connections and multiply control-pool/transfer pressure |
| S42 | low | confirmed | authorization-host-access | Peer storage analysis is globally limited to 2 concurrent scans with no per-principal share, so one peer can block analysis for all others |
| S43 | high | confirmed | share-server | Signaling and relay run in plaintext by default; a configured TLS endpoint silently falls back to plaintext |
| S44 | high | confirmed | share-server | Anyone who knows a Direct lookup ID can take over its server routing, swallow requests and force 'offline' |
| S45 | high | confirmed | share-server | Room routing trusts anyone who knows a room_id: roster disclosure, member eviction and forged room_left |
| S46 | medium | confirmed | share-server | Legacy direct_access_accepted: the accept/reject bit and message are unauthenticated and persistently flip Direct contacts |
| S47 | high | confirmed | share-server | Fixed, unauthenticated global ceilings let about 16 source addresses deny signaling to all users |
| S48 | medium | confirmed | share-server | The Iroh relay is open to any endpoint, and behind the documented TLS proxy its per-source cap limits the whole relay |
| S49 | medium | confirmed | share-server | Hello device_id is unauthenticated, so envelopes addressed by device are copied to impostors |
| S50 | medium | confirmed | share-server | Any registered client can block public discovery offers (pairing DoS) |
| S51 | medium | confirmed | share-server | Online PIN guessing against discovery offers is rate-limited but never capped |
| S52 | medium | confirmed | share-server | Unauthenticated server messages keep Android awake: power holds are requested before verification |
| S53 | medium | confirmed | share-server | The server's 128-message burst limit is below the client's normal publish burst, causing an endless reconnect loop |
| S54 | medium | confirmed | share-server | Silent WebSocket clients are never expired, so registration slots leak |
| S55 | low | confirmed | share-server | Presence device_name and fingerprint are not covered by the HMAC, and the MAC encoding is not injective |
| S56 | low | confirmed | share-server | The client's WebSocket transport accepts 64 MiB messages from the server |
| S57 | low | confirmed | share-server | Server-to-client messages over WebSocket wait up to 500 ms |
| S58 | medium | partially_confirmed | local-surface-at-rest | Linux and Android store Share private keys, relation secrets and remote passwords unencrypted at rest (no DPAPI/keystore equivalent) |
| S59 | low | partially_confirmed | local-surface-at-rest | Windows daemon IPC token/address/generation files are not owner- or ACL-verified (Linux strictly enforces uid+mode); token is the only gate to the full Share control plane |
| S60 | medium | confirmed | local-surface-at-rest | Pre-auth connection slots are exhaustible by any local process/app that reaches the loopback port, stalling the Share control plane (local DoS) |
| S61 | low | partially_confirmed | local-surface-at-rest | A single shared token grants any same-user process the full Share capability set, including remote command execution on paired peers |
| S62 | low | partially_confirmed | local-surface-at-rest | Share identity and profile files are created world-readable (0644) and app_data_dir is created with default umask, exposing the peer graph until the daemon tightens the directory |
| S63 | high | confirmed | local-surface-at-rest | Default Share export seeds the user's entire home directory as a shared root for new Direct peers and Rooms |
| S64 | medium | partially_confirmed | critic | Peer RemoveDir triggers unbounded native recursion with no depth or entry budget: an authorized peer can crash the host daemon (stack overflow) |
| S65 | medium | partially_confirmed | critic | Enabling "share saved connections" turns the host into an unrestricted read-write proxy into every credentialed remote server, for every peer |
| S66 | medium | confirmed | critic | An accepted peer can delay its own revocation (and block all profile changes) by keeping reciprocal-repair streams in flight |

## S01 Share signaling is plaintext by default: bare host:port, tcp://, ws:// and http:// are accepted silently and se-share-server cannot speak TLS

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/app/core/menus_settings.rs:38-39; share-server/Cargo.toml:14; native/src/daemon/os/shared/ipc_host_events.rs:44-47; native/src/daemon/os/shared/ipc_host_events.rs:121-131; native/src/daemon/os/shared/ipc_host_events.rs:248-266; android/app/src/main/java/app/smartexplorer/android/ui/settings/SettingsScreen.kt:179

**Beschreibung.** Only wss:// (and https://, rewritten to wss://) uses TLS (tungstenite client_tls with rustls/webpki roots, verified). A bare host:port and tcp:// open a raw TCP socket, ws:// and http:// (rewritten to ws://) open an unencrypted WebSocket. The CLI and Android validators accept tcp/ws/http/bare without warning or explicit opt-in, the desktop field hint names 'host:port' first, the docs list raw TCP first, and se-share-server itself has no TLS at all (default bind 0.0.0.0:51820, 'raw TCP + WebSocket upgrade'; TLS only through an external reverse proxy). The out-of-the-box deployment (run the server, enter server:51820) is therefore unencrypted and unauthenticated signaling. Server-originated DirectOffline, RoomLeft and Error messages are accepted without any proof.

**Fehlerszenario.** User starts se-share-server and enters 'server:51820'. A passive observer on the Wi-Fi/ISP path reads every Hello (device_id, device name, public key, fingerprint, all local IPv4 addresses), every presence (Iroh node id, relay URL, all candidate IP:ports, device name), all room ids and Direct lookup ids and the free-text message of tracked Direct requests; with the ids it can keep harvesting presences from the server (see the registration finding). An active MITM injects DirectOffline/RoomLeft (peers shown offline, opens refused), arbitrary Error text that the UI displays, and rewrites the unauthenticated device_name/fingerprint of valid presences. File contents, relation secrets and keys stay protected (Iroh QUIC end to end, HMAC/Ed25519), so the impact is metadata disclosure, DoS and UI spoofing.

**Richtung.** Make TLS the default: interpret a bare host[:port] as wss://, accept tcp/ws/http only with an explicit, visible opt-in (separate insecure scheme or setting plus UI/CLI warning) and show the effective transport security in Share status; give se-share-server native TLS (rustls with cert files or ACME, iroh-relay already has TlsConfig support) and make plaintext listening an explicit flag; reorder docs and hints to wss first.

**Gegenprüfung.** Every cited fact holds. connect_one sends ws/wss to connect_ws and tcp:// plus bare host:port to connect_tcp, which opens a raw socket (signal_connection.rs:71-95). normalize_signal_endpoint rewrites http:// to ws:// (341-350). Only a wss URI makes tungstenite's client_tls wrap the socket in rustls with webpki roots (native/Cargo.toml:109, signal_connection.rs:111-133). The CLI and Android validators accept tcp|ws|wss|http|https and bare endpoints with no warning (cli/share.rs:208, share_settings.rs:44). The desktop GUI writes the draft with no validation at all (menus_settings.rs:38-39), and its hint names host:port first (16). SHARE_SERVER.md:12-15 lists raw TCP first. se-share-server cannot do TLS: tungstenite is built with features=["handshake"] only (share-server/Cargo.toml:14), the default bind is 0.0.0.0:51820 (main.rs:68-71), and it logs 'raw TCP + WebSocket upgrade' (105-108).

What a passive observer reads: the Hello carries device_id, device name, public key, fingerprint and lan_ips (signal_connector.rs:37-50). Presences carry node id, relay, candidates and device name (signal_presence.rs:33-46). WatchDirect/JoinRoom carry the lookup and room ids (signal_publish.rs:35-65).

What an active attacker can inject: DirectOffline, RoomLeft and Error are forwarded without verification (signal_auth.rs:38-40, 90-95), and the daemon acts on them. DirectOffline clears contact.presence (ipc_host_events.rs:121-131), so endpoint_for_target refuses opens unless LAN evidence exists (service.rs:228-236). RoomLeft clears the member's presence (248-266; service.rs:268-271). Error becomes signal_error and a UI event (44-47). Android's placeholder is wss:// (SettingsScreen.kt:179), but plaintext is still accepted.

Severity: by the finding's own analysis, file data, relation secrets and keys stay protected by Iroh QUIC end to end plus HMAC/Ed25519. An on-path attacker can cause DoS by blocking traffic anyway. The design already treats the server as untrusted for exactly this data (share-server/src/main.rs:3-10), so plaintext extends server-level visibility and forgery of unsigned notices to on-path parties. That is metadata disclosure, DoS and UI spoofing: medium, not high.

## S02 Multi-endpoint server lists silently downgrade from wss:// to plaintext and mix TLS and plaintext relays

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/signal_connected.rs:61-64; native/src/share/core/session.rs:355-370

**Beschreibung.** SignalConnection::connect tries every comma/semicolon endpoint in order and returns the first that connects, for any error (TLS failure, reset, timeout), on every reconnect, without telling the user. The documented example 'wss://share.example.com/se-share, share.example.com:51820' therefore falls back to raw TCP whenever the TLS attempt fails. The relay list is derived from the same string: each endpoint becomes a relay URL (https://share.example.com and http://share.example.com:51821) and all of them go into RelayMode::custom; the published presence relay is the endpoint's current home relay.

**Fehlerszenario.** An active attacker on the client network resets TCP/443 or presents a bad certificate: the wss attempt fails, connect() moves on, registers over raw TCP:51820 and the client shows 'connected' while everything from the plaintext-signaling finding is exposed. Independently, with both relays configured, Iroh may choose http://share.example.com:51821 as home relay (assumption: Iroh selects the home relay by latency among configured relays; iroh 1.0.1 source is not vendored), so relayed traffic metadata is exposed even when signaling uses TLS.

**Richtung.** Never fall back from a TLS endpoint to a non-TLS one unless insecure transport was explicitly enabled; if any TLS endpoint is configured, drop non-TLS entries from both the signaling and the relay list; report and warn when the effective transport is plaintext; correct the documentation example.

**Gegenprüfung.** SignalConnection::connect tries the endpoints in order and returns the first that connects; any error (TLS failure, reset, timeout) moves on to the next (signal_connection.rs:53-69). It runs again on every (re)connect through connect_and_negotiate (signal_connector.rs:32-34). The only trace of which transport won is a Status line, 'Share-Server verbunden (<label>, ...)' (signal_connected.rs:61-64). That is logged, not a warning, so 'silently' is essentially right.

Relays: relay_urls_from_signal maps every endpoint (session.rs:333-348). wss becomes https with the path trimmed, and bare/tcp becomes http with port+1 (355-370). All of them go into RelayMode::custom (node.rs:132-138). The published relay is the first relay URL of endpoint.addr(), i.e. the home relay (endpoint_routes.rs:85-97). The finding's assumption about iroh is correct: iroh's net_report picks the relay with the lowest recent latency, keeping the current one unless the new one is clearly better (docs.rs iroh net_report.rs add_report_history_and_set_preferred_relay). So the plaintext relay can become the home relay with no attacker involved.

Severity lowered to low: the user must have entered a plaintext endpoint explicitly (opt-in, even though SHARE_SERVER.md:28-32 shows it as the example). The signaling downgrade also needs an active attacker. The impact is the same metadata exposure as findings 0 and 2.

## S03 Relay URLs default to plaintext http:// and the bundled relay has no TLS

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/Cargo.lock:3540-3543; vendor/iroh-relay-1.0.0/src/server.rs:155-166

**Beschreibung.** relay_url_from_endpoint turns bare host:port and tcp:// (port+1), ws:// and http:// into http:// relay URLs; parse_relay_url accepts http; SE_SHARE_RELAY_URL may be http; a peer's presence relay_url with http scheme is added as a dial route in endpoint_addr. se-share-server starts its relay with ServerConfig::default() plus relay settings only, i.e. without TLS, and logs an http:// URL. The iroh relay client disables TLS for http/ws URLs. Relay authentication then rests on the challenge signature over a plain WebSocket and the relay itself is not authenticated.

**Fehlerszenario.** Two peers behind NAT fall back to the relay. Every relay frame carries the destination/source EndpointId in clear (ClientToRelayMsg::Datagrams dst_endpoint_id), so any network observer sees which devices talk, when and how much; an on-path attacker can impersonate the relay and blackhole or delay traffic. QUIC payload stays end-to-end encrypted and peer-authenticated, so file data is not exposed.

**Richtung.** Derive https:// relay URLs by default and reject http relays (own config, env override, peer presence) unless the explicit insecure opt-in is set; add TLS to se-share-server's relay (TlsConfig with certificate or ACME) or refuse to derive an http relay and document that it must be published only behind a TLS proxy.

**Gegenprüfung.** relay_url_from_endpoint produces http:// URLs for ws://, http://, tcp:// and bare endpoints (session.rs:355-370), and parse_relay_url accepts http (350-353). SE_SHARE_RELAY_URL goes through the same parser (transport_options.rs:8-12). A peer's presence relay_url is added as a dial route (session.rs:275-277). se-share-server starts the relay with RelayConfig::new, which sets tls: None (vendor server.rs:155-166; relay.rs:113-114,152), and logs an http:// URL (relay.rs:121-125). The vendored iroh-relay client disables TLS for http/ws (client/tls.rs:97-105). Datagrams frames carry dst_endpoint_id (protos/relay.rs:166-180).

Location caveat: the native client links crates.io iroh-relay 1.0.1 (Cargo.lock:3540-3543), not the vendored 1.0.0 copy, which only the server is patched to use. The same scheme rule is presumably unchanged.

Scope: a wss endpoint derives an https relay, so this applies exactly when the signaling endpoint is plaintext. It is the relay half of finding 0's default deployment. Payloads stay QUIC end to end, but who-talks-to-whom, timing and volume become visible to passive observers. Medium is acceptable.

## S04 Default export is the whole home (Android: whole shared storage), read-write, for every Direct code holder, every reciprocal pair and every room member

- Schwere: high · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: android/app/src/main/java/app/smartexplorer/android/core/InitConfig.kt:38; native/src/mobile/os/shared/domains/share_state.rs:395-399; native/src/share/core/server_fs.rs:117-262; native/src/share/core/configuration_runtime.rs:143-181; native/src/share/core/direct_reciprocal_transport.rs:63-189; native/src/share/core/direct_reciprocal.rs:339-362

**Beschreibung.** Authorization default rather than transport, reported because the request is 'secure by default'. A newly created profile seeds default_direct_exports with the user's home; every joined or created room copies that export; ShareExportConfig/SharedRoot have no read-only mode, so writes, renames and deletes are served inside it. The host serves default_direct_exports to any authenticated Direct peer and room.exports to any room member. The first tracked request from any holder of the Direct code is decided Accepted automatically, the reciprocal repair installs an Accepted grant on the requesting side as well, and new room device ids are added unblocked automatically. On Android the home is the primary volume (/storage/emulated/0).

**Fehlerszenario.** User A adds B's Direct code only to browse B's files. B auto-accepts, the reciprocal repair installs an Accepted grant for B on A, and B can now read, overwrite and delete A's entire home (on Android all shared storage) without A ever choosing to share anything. Likewise a room code forwarded in a chat lets every recipient join and read/modify/delete the home of every member who kept the default.

**Richtung.** Default to deny-all (empty exports) or a dedicated share folder, read-only unless the user enables write per export (add an enforced read-only flag); start rooms with no exports; let reciprocal repair create the contact but not an Accepted grant unless the user opted in; consider a confirmation for the first request of a new device.

**Gegenprüfung.** The default export is the home directory, and it is writable:
- When the profile file is missing, the profile is seeded with the home as an export (profile_persistence.rs:63-71).
- Room join and room creation copy that export (229-231; cli/share.rs:145-150; mobile share_peers.rs:84-104).
- SharedRoot and ShareExportConfig have no access mode (fs.rs:17-27). The host serves Write, WriteNew, Rename, RemoveFile, RemoveDir and PutBatch under them (server_fs.rs:117-262).
- authorize_state returns default_direct_exports for an Accepted grant with a valid proof, and room.exports for any unblocked member (session.rs:185-253).

Direct access is granted automatically, in both directions:
- Incoming tracked requests are decided Accepted when no prior grant exists (ipc_host_direct_events.rs:221-238, applied automatically at 70-80). Denial happens only for ignored or removed peers (legacy_direct_request.rs:195-235).
- For every Accepted, auto_connect contact the worker schedules a reciprocal repair (configuration_runtime.rs:143-181). Both initiator and receiver persist (direct_reciprocal_transport.rs:63-189), installing an Accepted grant for the peer (direct_reciprocal.rs:339-362). So adding B's code opens A to B as well.
- New room device ids are inserted unblocked (ipc_host_events.rs:468-482).

Location fix: the cited support_dirs.rs:113 is a test fixture. In production the Android home is the primary volume path (InitConfig.kt:38 -> share_state.rs:395-399).

Mitigations exist: the UI shows 'Freigegeben: <summary>' next to the code (share_direct_ui.rs:62-66), and the reciprocal behaviour is documented (SHARE_SERVER.md:62-69). Still, nothing is read-only and nothing asks the requesting side for consent. This is an authorization default, not transport encryption, but it contradicts 'secure by default'. High stands.

## S05 Blocking a room member is not a revocation: the room secret never rotates and new identities are auto-admitted

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/session.rs:226-236; native/src/share/core/identity.rs:196-235

**Beschreibung.** Room membership is proven only by knowledge of the static room secret (presence HMAC and session proof). The blocked flag applies to one device_id. Any verified presence with a new device_id is inserted as an unblocked member and then admitted by authorize_state. No code path re-keys or rotates a room (rg 'rotat' only finds Direct code rotation).

**Fehlerszenario.** The user blocks device X in room R. X regenerates its Share identity (new device_id and Iroh key) or another install uses the same room code: its presence verifies with the unchanged secret, it is added as a new unblocked member and its session proof is accepted, restoring full access to the room's exports.

**Richtung.** Add room re-keying (new room id/secret distributed to the remaining members over authenticated Iroh sessions or a new code, old secret rejected) triggered by block/remove; optionally require approval of new member identities instead of automatic admission.

**Gegenprüfung.** Room presences are authenticated only by an HMAC under the static room secret (signal_auth.rs:279-320). upsert_room_member matches members by device_id alone and inserts any new device_id with blocked: false (ipc_host_events.rs:445-483). authorize_state looks up the member by device_id && !blocked, then checks node and public key against that same attacker-supplied entry (session.rs:226-236). Every other blocked check is also keyed by device_id (service.rs:265, peer_endpoint_source.rs:156, exec_auth.rs:96). new_room_code generates a fixed id plus secret (profiles.rs:151-157), and the only rotation API is for the Direct code (identity.rs:196-235).

The bypass is easier than the finding describes. device_id is not bound to any key, so a blocked device does not need to regenerate its identity. It can put a fresh device_id in its presence and PeerHello while keeping its real node and key, and it is admitted. Medium is reasonable for an insider holding the room secret. Combined with finding 3, it regains read-write access to members' homes.

## S06 Presence MAC excludes device_name and fingerprint; receivers store and display them

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/crypto.rs:136-153; native/src/share/core/signal_presence.rs:21-46; native/src/daemon/os/shared/ipc_host_events.rs:460-461; native/src/daemon/os/shared/ipc_host_events.rs:469-476; native/src/share/core/session.rs:234-236; native/src/share/core/exec_auth.rs:142-151; native/src/share/core/exec_targets.rs:77; native/src/share/core/service.rs:280; native/src/share/core/peer_endpoint_source.rs:121-139

**Beschreibung.** presence_payload authenticates kind, relation_id, device_id, public_key, node_id, relay_url, candidates, expires_at and nonce, but not device_name and not fingerprint (and has no domain-separation/version prefix). upsert_room_member copies device_name and fingerprint into the member without checking the fingerprint against the public key; Direct contacts keep presence.fingerprint, which later gates mounts (peer_endpoint_source) and Exec (validate_server). Room member names feed the room list, mount labels and the Exec target list.

**Fehlerszenario.** A malicious Share server (declared untrusted) or a MITM on plaintext signaling swaps the names of two room members: the user mounts, uploads to, or enables Exec (unrestricted code execution) for the device labelled 'Laptop' that is actually the other member. Altering the fingerprint field makes hosts reject that member ('Raumgeraet hat ungueltigen Fingerprint') and makes Exec fail - targeted DoS until a clean presence arrives.

**Richtung.** Add device_name and fingerprint plus a versioned domain-separation prefix to the presence MAC; verify fingerprint == public_fingerprint(public_key) on receipt; for old peers do not store/display unauthenticated names.

**Gegenprüfung.** presence_payload MACs kind, relation_id, device_id, public_key, node_id, relay_url, candidates, expires_at and nonce only (crypto.rs:136-153). device_name and fingerprint travel outside the MAC (signal_presence.rs:33-46). The verifiers never check presence.fingerprint against public_key; the Direct path checks only public_key against the pinned expected_fingerprint (signal_auth.rs:248).

The unauthenticated fields are used:
- upsert_room_member copies device_name and fingerprint (ipc_host_events.rs:460-461, 469-476).
- A wrong member fingerprint makes the host reject that member (session.rs:234-236).
- Direct mount reopen compares contact.expected_fingerprint with presence.fingerprint (peer_endpoint_source.rs:121-139).
- Exec compares the server's fingerprint with presence.fingerprint (exec_auth.rs:142-151).
- Names feed the Exec target list (exec_targets.rs:77) and mount labels (service.rs:280).

By contrast, the tracked-direct transcripts do authenticate device_name and fingerprint (direct_transcript.rs:250-254). The missing domain prefix is only a nit: payloads start with the kind, and the other MACs under the same secrets use distinct prefixes (session.rs:318, direct_transcript.rs:9). A tampering server or MITM can mislabel members (UI spoofing that could lead a user to grant Exec to the wrong device) or cause targeted DoS. Medium is right.

## S07 Unauthenticated peers can exhaust the 64 pre-authentication handshake slots and lock out all incoming Share sessions

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/node_accept.rs:16-43; native/src/share/core/node.rs:34-35; native/src/share/core/node.rs:167-173; native/src/share/core/handshake_limits.rs:43-70; native/src/share/core/server.rs:34; native/src/share/core/server.rs:89-103; native/src/share/core/exec_server.rs:24

**Beschreibung.** The accept loop takes one of 64 global slots before the QUIC handshake completes and keeps it until PeerHelloOk; the per-peer cap of 4 is keyed on the remote EndpointId, which an attacker picks freely (fresh Ed25519 key per connection). The FS and Exec hello deadline is 20 s. remote_id is not checked against known grants, room members or contacts before the slot is taken.

**Fehlerszenario.** Anyone who knows the node id (published in every presence, visible to the server and to plaintext observers) or a candidate address opens about 64 connections with fresh keys, directly or through the open relay, and never sends PeerHello, renewing them as they expire (a few per second). Every legitimate Direct/Room/Exec connection hits incoming.refuse(): mounts, transfers and syncs to that device fail for the duration.

**Richtung.** Take the global permit only after the TLS handshake; give EndpointIds present in grants/room members/contacts a reserved pool and unknown ids a small separate pool with a short (2-5 s) hello deadline; add per-source-address limits on direct paths.

**Gegenprüfung.** The accept loop takes one of 64 global permits before incoming.await, i.e. before the QUIC handshake (node_accept.rs:16-31; node.rs:34-35, 167-173). The per-peer limiter (4 per remote id, 64 ids) is keyed on connection.remote_id() (node_accept.rs:32-42; handshake_limits.rs:43-70), which the client picks freely. The global permit is dropped only after PeerHelloOk or Exec HelloOk (server.rs:141; exec_server.rs:116), or when the 20 s application handshake deadline fails (server.rs:34, 89-103; exec_server.rs:24, 34-40). The pre-handshake phase is bounded only by the 20 s QUIC idle timeout (keepalive.rs:7, 107-109). No check against grants, members or contacts happens before the hello.

Roughly 64 stalled connections every 20 s (about 3 per second) are enough. They can come through the open relay, whose AccessControl admits anyone up to capacity (relay.rs:200-219), or through a known candidate address. With the slots full, every legitimate incoming Direct, Room and Exec connection is refused. Medium is right.

## S08 Signaling registrations are unauthenticated: lookup ids and room ids act as bearer tokens for presence data and entry replacement

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: share-server/src/tracked_direct.rs:243-258; share-server/src/state.rs:251-262

**Beschreibung.** Hello registers a self-asserted device_id; public_key and fingerprint are ignored and no proof of key possession is requested. Any registered client can watch any lookup_id (immediately receiving the stored presence), join any room_id (receiving the full roster), publish a presence for any lookup_id (last writer wins) and evict another client's room entry by joining with the same device_id. Presence bodies (node id, all candidate IPs, relay, device name) are plaintext JSON for the server and for anyone holding an id.

**Fehlerszenario.** A former member, the operator, or anyone who sniffed plaintext signaling joins room R from a throwaway client and continuously receives every member's IP addresses, device names and online times. Joining with a victim's device_id removes the victim's server-side membership (replaced_client), and PublishDirect with a victim's lookup_id overwrites its presence with an invalid one that watchers drop - the victim appears offline.

**Richtung.** Require proof of possession at Hello (Ed25519 signature over a server challenge with the node key) and bind device_id to that key; bind lookup_id/room entries to the first proven key or to an access token derived from the relation secret (e.g. HMAC(secret, 'server-access'|id)) so only relation members can watch/join/publish; encrypt presence bodies with a relation-secret-derived AEAD key so the server routes opaque blobs.

**Gegenprüfung.** The Hello's public_key and fingerprint are discarded. register_client stores the self-asserted device_id with no proof of key possession and no uniqueness check (transport.rs:120-155, 199-237; state.rs:60-101).

- join_room only requires presence.device_id == client.device_id. It sends the full roster to the joiner (state.rs:123-147) and drops the previous holder's membership (157-166).
- publish inserts any lookup_id, last writer wins (tracked_direct.rs:206-241).
- watch returns the stored presence to any watcher (260-305).

'Appears offline' is reachable more directly than the finding says. A client that publishes for a victim's lookup_id and then unpublishes or disconnects triggers notify_offline, which sends DirectOffline to every watcher (tracked_direct.rs:243-258; state.rs:251-262). Clients apply DirectOffline unverified (finding 0).

The ids act as bearer tokens known to the operator, to former contacts and members, and (with plaintext signaling) to observers. Medium is right.

## S09 LAN uplink sharing (opt-in) is started by unauthenticated mDNS announcements

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux
- Stellen: native/src/net/core/link_facts.rs:77-82; native/src/net/os/windows/uplink_helper.rs:254-273

**Beschreibung.** The uplink policy treats any mDNS record whose 16-hex id matches a paired contact as that peer, trusts its advisory up=0 flag and its announced A/AAAA addresses, and maps them to local interfaces by /24 prefix (or to every router-less link for link-local-only IPv6). The id is not secret: the real peer broadcasts it on every network and anyone knowing the node id can compute it. After 5 s the host enables Windows ICS or NetworkManager ipv4.method=shared (NAT plus DHCP server) on that interface. The design calls LAN announcements routing evidence only, but here they authorize a privileged network change.

**Fehlerszenario.** With uplink sharing enabled, an attacker replays the paired phone's announcement (captured on another network) with up=0 and an address inside the /24 of a router-less host interface (static lab/industrial LAN, host-only adapter). The host starts NAT and a DHCP server on that segment (rogue DHCP for other devices, internet bridging for the attacker) and keeps it while spoofed announcements continue.

**Richtung.** Start sharing only on facts from an authenticated Iroh session with the paired peer over that link (peer reports 'no uplink' inside the authenticated channel and the direct path's local interface is confirmed), use the receiving interface rather than announced addresses, and keep LAN announcements as dial hints only.

**Gegenprüfung.** sighting_from trusts the TXT 'up' flag and the announced A/AAAA addresses (lan_presence.rs:144-173). match_sighting needs only an Accepted contact whose hashed_lan_id matches; that hash is unkeyed, computed from the node id (lan_presence_match.rs:20-30, 87-102). peer_interfaces maps a sighting to local interfaces by /24 or link-local prefix (107-143). So a spoofed packet received on any interface can name a router-less interface purely by the address it claims.

lan_runtime feeds these sightings to UplinkPolicy (lan_runtime.rs:98-111, 154-180). The policy returns Start after START_DEBOUNCE_SECS=5 (lan_uplink_policy.rs:6, 122-183) and keeps sharing while spoofed sightings continue (92-120, 90 s grace). RouterLess means no gateway and no DHCP lease (link_facts.rs:77-82). The elevated Windows helper re-checks only link classes, never the peer (uplink_helper.rs:254-273). The feature is opt-in (lan_settings.rs:11) and exists on Windows and Linux only (net/mod.rs:59-68). Medium is right.

## S10 Elevated uplink helper task runs a user-writable executable (silent UAC bypass) and executes a user-writable setup script

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows
- Stellen: native/installer.nsi:290-329

**Beschreibung.** The one-time setup registers Scheduled Task 'Smart Explorer LAN-Uplink' with RunLevel Highest whose action is current_exe(); the per-user installer puts that exe under LOCALAPPDATA/Programs/Smart Explorer, writable by the unelevated user, and the daemon starts the task unelevated by design (Start-ScheduledTask). The setup script is written to the user-writable app-data directory and then executed elevated via Start-Process -Verb RunAs. Adjacent to the network dimension (net/ scope, triggered by the LAN feature).

**Fehlerszenario.** Any medium-integrity process of the user replaces se.exe or plants a DLL in the install directory and runs Start-ScheduledTask for that task: its code runs with the user's full administrator token without a UAC prompt. During setup the same process can swap setup.ps1 between the write and the elevated execution, so the user's UAC consent runs attacker code.

**Richtung.** Point the task at a helper copied to an admin-only location and verify its SHA-256 before each run (as AGENTS.md requires for elevated helpers), or perform the ICS switch through a service with a narrow interface; pass the setup script inline or verify its hash after elevation.

**Gegenprüfung.** setup_once registers the task 'Smart Explorer LAN-Uplink' with RunLevel Highest, executing current_exe() (uplink_helper.rs:90-106). It writes setup.ps1 into the user-writable app-data directory and runs it through Start-Process -Verb RunAs (107-124), which leaves a window to swap the script while the consent prompt is open. run_via_task then starts the task unelevated via Start-ScheduledTask (175-178). The install directory is per-user under %LOCALAPPDATA%\Programs (installer.nsi:59-60), so the executable is user-writable.

Nothing ever unregisters the task: no other code references TASK_NAME, and the NSIS uninstall section (290-329) does not remove it. The elevation path therefore outlives disabling the feature and even uninstalling.

For administrator accounts with UAC split tokens, this is a silent jump from medium to high integrity. UAC is not a formal security boundary and the feature is opt-in, so medium fits.

## S11 Linux polkit rule grants permanent passwordless system-wide NetworkManager control

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Linux
- Stellen: native/src/net/os/linux_os/uplink_polkit.rs:9-21; native/src/net/os/linux_os/uplink_polkit.rs:47-70

**Beschreibung.** The rule returns YES for org.freedesktop.NetworkManager.settings.modify.system and network-control for the user with no subject.local/subject.active condition, and nothing removes it when uplink sharing is disabled. The rule text is staged in the user-writable app-data directory and installed by pkexec as root.

**Fehlerszenario.** After enabling uplink sharing once, any process of that user, including an SSH session or background malware, can silently create or modify system-wide NM connections (DNS, routes, VPN, Wi-Fi) for all users; a same-user process can swap the staged file between write and pkexec to install an arbitrary polkit rule under the user's single authentication.

**Richtung.** Require subject.local && subject.active, remove the rule when the feature is turned off, and write the rule from a root-side process (or a root-owned staging path) instead of a user-writable file.

**Gegenprüfung.** The rule returns YES for settings.modify.system and network-control for subject.user, with no subject.local or subject.active condition (uplink_polkit.rs:9-21). The rule text is staged in the user-writable app-data directory and then installed by pkexec install (47-64), which leaves a same-user swap window. Removal code does not exist: a grep finds only RULE_PATH and rule_installed. The impact needs a process already running as that user. Low is right.

## S12 FS handshake reveals the exact authorization failure to unauthenticated peers

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/server.rs:118-133; native/src/share/core/session.rs:185-255

**Beschreibung.** On a failed PeerHello the host sends the specific reason to the not-yet-authenticated peer ('Unbekannter Raum', 'Geraet nicht im Raum', 'Raumgeraet hat Identitaetskonflikt', 'Direktverbindung ist offline', 'Direktfreigabe nicht akzeptiert', 'Session-Proof ungueltig'). The Exec ALPN deliberately answers a uniform 'exec authentication failed'.

**Fehlerszenario.** An attacker who knows a host's node id connects with arbitrary keys and probes room ids and device ids; the distinct messages reveal whether the host has an active room with that id, whether a device id is a non-blocked member and whether Direct sharing is online.

**Richtung.** Send one generic denial on the wire and keep the detailed reason in local diagnostics, as the Exec path already does.

**Gegenprüfung.** Before authentication, the host sends fs_error::response(&error) (server.rs:118-133), whose msg is error.to_string() (fs_error.rs:7-12). authorize_state's texts are distinct (session.rs:189-255): unknown room, not in room, identity conflict, Direct offline, not accepted, invalid proof. Exec answers a uniform 'exec authentication failed' (exec_server.rs:65-75).

Probing does require a relation id the attacker already knows. Room ids are 96-bit random values (profiles.rs:151-157), so this confirms ids learned elsewhere rather than discovering them. Low is right.

## S13 Signaling Hello sends all local IPv4 addresses and device name that the server never uses

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/signal_connector.rs:37-50; native/src/share/os/shared/system.rs:1-24; share-server/src/transport.rs:120-129

**Beschreibung.** Every Hello carries lan: lan_ips() (all non-loopback IPv4 interface addresses plus the default-route source address) and the device name; the server discards both (lan: _, device_name: _).

**Fehlerszenario.** Over plaintext signaling, or at the operator, the internal address plan of every device (VPN, container, corporate ranges) is disclosed on each connect without any function.

**Richtung.** Send an empty lan list (field kept for wire compatibility) and only the fields the server consumes.

**Gegenprüfung.** The Hello includes lan: lan_ips() and device_name (signal_connector.rs:37-50). lan_ips lists every non-loopback IPv4 address plus the default-route source address (system.rs:1-24). The server discards device_name, listen_port, lan, public_key and fingerprint (transport.rs:120-129, 199-208).

The extra exposure is modest. Every presence, which the server stores and forwards, already carries device_name and the Iroh direct-address candidates (signal_presence.rs:17-46; endpoint_routes.rs:85-97), and those typically include local interface addresses. Low is right.

## S14 LAN presence is on by default on every network with a stable trackable id and hostname; unauthenticated sightings are unbounded and overwrite routes

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/daemon/os/shared/lan_runtime.rs:230-251; native/src/daemon/os/shared/lan_runtime.rs:277-280

**Beschreibung.** presence_enabled defaults to true, so every device running Share announces se-<id> with its host/device name and all addresses on every network it joins, including public Wi-Fi and with no paired contacts. The id is a deterministic hash of the node id, recognizable by the server operator, room members and plaintext observers. Received sightings go into an unbounded channel and map and are re-matched every tick against all contacts (one SHA-256 per contact per sighting); a sighting matching a contact replaces its lan_candidates.

**Fehlerszenario.** On a cafe network anyone sees the laptop's hostname and stable id and can recognize the device across locations; a LAN attacker flooding random ids grows memory and per-tick CPU; a spoof with a paired contact's id replaces the real LAN route so offline-mode dialing fails (identity is still protected by the node pin).

**Richtung.** Announce only while accepted Direct contacts exist and preferably only on private networks; announce a rotating keyed id (e.g. HMAC of relation material and an epoch) instead of a stable node-id hash; do not publish the hostname; cap the sighting map/channel; merge rather than replace candidates.

**Gegenprüfung.** presence_enabled defaults to true (lan_settings.rs:8-9, 24-31). Whenever the Share service has bound ports, the device announces se-<hashed_id> with host '<sanitized hostname>.local.' (lan_presence.rs:103-124; lan_runtime.rs:230-251), whether or not it has contacts. The id is an unkeyed hash of the node id (lan_presence_match.rs:20-30), so anyone who knows the node id can recognize it.

One nuance on 'unbounded': sightings pass through an unbounded channel (lan_presence.rs:47) into a map with no size cap (lan_runtime.rs:263-268), but entries expire after the 150 s TTL (277-280). Memory is therefore bounded by flood rate times 150 s rather than truly unbounded. reconcile computes one SHA-256 per contact per sighting (lan_presence_match.rs:92-96).

A spoof with a paired contact's id replaces lan_candidates and can also flip the status to Available (ipc_host_events.rs:269-292). Without a current server presence, effective_presence dials only those LAN candidates (lan_presence_match.rs:174-201). Identity still rests on the node pin. Low is right.

## S15 Firewall rule opens the whole executable on all network profiles and is created automatically

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Windows
- Stellen: native/src/share/os/windows/system.rs:15-66; native/src/share/os/windows/system.rs:68-78; native/src/share/core/service.rs:310-319

**Beschreibung.** Every ShareService start deletes and re-adds 'Smart Explorer Share Peer Listener' as a program-wide inbound allow (all protocols and ports) with profile=any, and on failure launches an elevated netsh through UAC without the user asking for it.

**Fehlerszenario.** On a Public-profile network every socket the executable binds on all interfaces is reachable (today the Iroh UDP endpoint and mDNS, the pre-auth surfaces of the slot-exhaustion and LAN findings); any later non-loopback listener in the same binary becomes reachable from untrusted networks without review.

**Richtung.** Restrict the rule to UDP and the bound Iroh port(s), profile private/domain by default with an explicit opt-in for public networks, and do not raise UAC unattended.

**Gegenprüfung.** On every ShareService start (service.rs:310-319), ensure_firewall_rule_for deletes and re-adds a program-wide inbound allow with profile=any (windows/system.rs:15-45). If adding fails, it launches an elevated netsh through Start-Process -Verb RunAs, once per process (system.rs:8, 50-55, 68-102). Because the rule is re-created unconditionally and netsh needs administrator rights, an unelevated daemon will request UAC on every process start.

Today the binary's only non-loopback sockets are the Iroh UDP endpoint and mDNS; every other bind is 127.0.0.1. Quick Share is GUI-only mDNS discovery with no listener and is not reachable from Share flows (quickshare.rs:38-99; used only from app/core/quickshare_ui.rs). Low is right.

## S16 Presence replay cache is flushed wholesale at 4096 entries

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/signal_auth.rs:322-327

**Beschreibung.** remember_nonce clears the entire seen-nonce set (shared by Direct, room, request and accepted presences) once it exceeds 4096 entries; nothing else expires entries.

**Fehlerszenario.** A busy room or a room member (secret holder) produces more than 4096 verified presences; the set is cleared and a captured older presence still inside its validity window (up to 15 min) is accepted again, rolling a peer back to stale candidates/relay and causing connection failures until the next refresh.

**Richtung.** Store each nonce with its presence expiry and evict only expired entries, and/or reject presences older than the newest accepted one per sender.

**Gegenprüfung.** remember_nonce clears the whole set once it holds more than 4096 entries (signal_auth.rs:322-327). seen_nonces is a single set shared by the direct, direct-request, direct-accepted and room replay keys (types.rs:469; service.rs:330), and nothing else prunes it.

Honest presences live 300 s (signal_presence.rs:20); a secret holder can mint presences valid up to 15 min (types.rs:130-138). After a flush, the server or a MITM can replay a still-valid presence, rolling a peer back to stale candidates or relay until its next refresh (60 s). Low is right.

## S17 node_id is not bound to public_key; unpinned legacy contacts/members dial the node id from presence

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/daemon/os/shared/ipc_host_events.rs:104-106; native/src/share/core/direct_reciprocal.rs:286-288; native/src/share/core/profiles.rs:193-222

**Beschreibung.** Presence verification checks the public key against the pinned fingerprint but checks node_id only when a node pin exists; nothing requires node_id to equal (or be signed by) public_key, although current identities use the same key. The host answers PeerHelloOk without any proof of relation membership, so the client relies solely on the dialed node id. SE-D3 codes always pin the node id, so only legacy contacts and never-pinned room members are affected.

**Fehlerszenario.** Legacy Direct contact with empty expected_node_id: another holder of that host's Direct secret publishes a presence with the host's real public key (fingerprint matches) and its own node id; the client dials it, gets PeerHelloOk, uploads files to the attacker or reads fake content.

**Richtung.** Require node_id == public_key for v3 identities (or a node-key signature over the presence), refuse to dial unpinned legacy peers until re-paired, and add a host proof (HMAC over the client nonce) to PeerHelloOk for defense in depth.

**Gegenprüfung.** The cited code is as described:
- verify_direct_presence checks the fingerprint against public_key, and node_id only when a pin exists (signal_auth.rs:248-253).
- Nothing binds node_id to public_key; they are equal only for new identities (identity.rs:333-341).
- endpoint_for_target falls back to presence.node_id (service.rs:239-243). Rooms use member.node_id, which also comes from presence (286).
- The client accepts PeerHelloOk without any host proof (node_sessions.rs:384-395).

The window is narrower than the finding suggests:
- DirectCode::parse accepts only SE-D3 codes, which must carry a node id (profiles.rs:193-222).
- The first verified DirectAvailable pins an empty expected_node_id, trust-on-first-use (ipc_host_events.rs:104-106).
- The reciprocal apply also pins it (direct_reciprocal.rs:286-288).
- Room members keep their node id once it is non-empty (ipc_host_events.rs:451-452).

Only a legacy contact whose very first post-upgrade presence comes from another holder of the host's Direct secret is affected, or a legacy room member with an empty node id. Low is right.

## S18 ws:// and http:// endpoints derive the relay on the signaling port, so relay fallback silently fails

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/session.rs:355-370; share-server/src/relay.rs:280-291; share-server/src/transport.rs:79-90

**Beschreibung.** For tcp:// and bare host:port the derived relay uses port+1 (the server's default relay bind), but ws://host:port and http://host:port keep the signaling port. se-share-server accepts any WebSocket upgrade on the signaling port and then waits for a JSON Hello, so the iroh relay handshake there never completes.

**Fehlerszenario.** Client configured 'ws://server:51820' (no proxy): signaling works but the home relay never connects; peers behind NAT cannot reach each other and the relay status keeps flapping.

**Richtung.** Apply the port+1 rule to ws:// endpoints with an explicit port (or require an explicit relay URL) and validate at configure time that the derived relay answers the relay probe.

**Gegenprüfung.** relay_url_from_endpoint keeps the signaling port for ws:// and http:// (session.rs:358-361) but adds 1 for tcp:// and bare endpoints (362-368; relay_tcp_addr 372-384). The bundled relay binds signal port + 1 by default (relay.rs:280-291). On the signaling port, se-share-server treats anything starting with 'G' as a signaling WebSocket and waits for a JSON Hello (transport.rs:79-90, 167-198), so an iroh relay probe or upgrade there can never complete.

With ws://server:51820, signaling works but no home relay is ever established. home_relay_connected then reports Some(false), and check_home_relay keeps triggering network-change notifications (node_wake.rs:130-133; signal_session.rs:58-69), which matches the 'flapping' description.

Behind a reverse proxy that routes /relay, the derivation works. Only direct ws/http use of the bundled server is affected. Low is right.

## S19 Removed or revoked Direct peers can get access back by using a new device_id, because the device-wide Direct code is accepted automatically and the removal record checks only the device_id

- Schwere: high · Urteil: confirmed · Kategorie: security · Plattformen: 
- Stellen: native/src/share/core/removed_direct_peers.rs:38-40; native/src/share/core/legacy_direct_request.rs:195-239; native/src/daemon/os/shared/ipc_host_direct_events.rs:221-238; native/src/share/core/legacy_direct_request_mutations.rs:92-100; native/src/share/core/legacy_direct_request_mutations.rs:395-399; native/src/share/core/identity.rs:52-54; native/src/share/core/identity.rs:74-89; native/src/share/core/identity.rs:440-450; native/src/share/core/identity_repair.rs:97-157; native/src/share/core/direct_reciprocal.rs:218-231; native/src/share/os/shared/removal.rs:55-60; README.md:586-592; docs/SHARE_SERVER.md:350-356

**Beschreibung.** Each device has exactly one Direct secret. ShareIdentity::direct_code() puts it into every Direct code, and every PIN-discovery bundle (direct_peer_from_identity) and every reciprocal-repair Hello/Offer carries the same material. A tracked request is HMAC-signed with that secret and signed by any Iroh key the requester chooses. If no grant exists for that device_id, the request is accepted automatically. The legacy path accepts the same way (install_grant: true, source AuthenticatedSecretPossession). The only durable denial after 'Entfernen'/'Eintrag loeschen' is RemovedDirectPeer, and RemovedDirectPeer::matches compares only device_id. device_id is a string the device asserts about itself; ShareIdentity validate_disk only checks that it is non-empty. All other denials (ignored grant, user rejection selector, tombstones) require the exact (device_id, key, node) tuple. A former peer that still holds the code can therefore come back under any new device_id, with the same key or a new one. The UI headline and README/SHARE_SERVER.md promise that automatic re-pairing is blocked ('kann sich nicht mehr automatisch wieder eintragen ... ueber eine neue Anfrage mit dem bekannten Direkt-Code'). The only working cut-off is regenerating the Direct code, which revokes every peer (invalidate_all_direct_grants).

**Fehlerszenario.** 1. Alice removes Bob. Headline: '... entfernt ...; automatische Wiederkopplung gesperrt'. Bob's device still has Alice's code (his contact secret plus her lookup, fingerprint and node).
2. Bob gets a new device_id. He can edit device_id in share_identity.json, or delete his Iroh secret and let repair_missing create a new device_id and key.
3. Bob deletes his Alice contact, re-adds her code and sends a new tracked request.
4. Alice's daemon (ensure_authenticated_request_decision) runs direct_auto_accept_denied: false, because removed_peer is matched by device_id only. grant_for(new id) returns None, so the result is DirectDecisionKind::Accepted. project_grant pushes an Accepted DirectGrant.
5. Reciprocal repair then re-creates the contact.
Result: within seconds Bob has read/write/delete access to Alice's default exports again (see the default-export finding), with no prompt. The legacy DirectAccessRequest path gives the same result through authenticated_decision(None) => Accepted.

**Richtung.** Stop using one device-wide secret as a bearer capability:
- Issue a separate relation secret per paired peer, at pairing or repair time. Removal can then revoke that one secret without disturbing other peers.
- Treat the published Direct code as a short-lived or single-use invitation.
- Make automatic acceptance opt-in, or require user approval for any identity not previously approved.
- Key removal and denial records on the public key and node id as well as the device_id, and on the relation material that was given out.
- Until then, offer or force a Direct-code rotation when removing a peer, and correct the UI and docs claims.

**Gegenprüfung.** Code matches the claim. RemovedDirectPeer::matches compares only device_id (removed_direct_peers.rs:38-40). Every other auto-accept denial in direct_auto_accept_denied needs the exact tuple: ignored grant at legacy_direct_request.rs:200-206, ignored contact at :207-213, tombstone `requester == *peer` at :214-218, and the selector over device_id/key/node at :219-231. Only removed_peer at :232 uses device_id alone. ensure_authenticated_request_decision maps `grant_for(new id) == None` to Accepted (ipc_host_direct_events.rs:224-237), and project_grant then pushes an Accepted grant (direct_ledger_projection.rs:111-129). The legacy path does the same: authenticated_decision None => Accepted with install_grant:true (legacy_direct_request_mutations.rs:395-399, applied at :98-100). Identity conflicts are also keyed per device_id (legacy_direct_request_reconciliation.rs:154-177, 256-261), so a new id raises no conflict. device_id is self-chosen: validate_disk only checks it is non-empty (identity.rs:440-450). It is even easier than the report says: deleting share_identity.json makes load_or_create_with call create() (identity.rs:52-54), which reuses the stored Iroh key and mints a new UUID (identity.rs:72-89). An honest reinstall that clears app data, followed by re-adding the remembered code, has the same effect. Reciprocal repair is blocked only by removed_direct_peer(identity), which is again device_id-only (direct_reciprocal.rs:218-231), so the contact is recreated as claimed. This breaks the README promise at README.md:588-592 and the 'automatische Wiederkopplung gesperrt' headline (removal.rs:56-60). I rate it high, not critical: the attacker must already hold the device-wide Direct secret, so the risk is a former trusted peer getting back in, not access for strangers. Code rotation is the only real cut-off, and it has its own breakage (finding #4).

## S20 PIN discovery lets strangers pair by default: the desktop GUI publishes with an empty PIN, there is no strength minimum and no lockout, offers stay open after a successful pairing, and every client of the server sees the offer list

- Schwere: critical · Urteil: confirmed · Kategorie: security · Plattformen: 
- Stellen: native/src/share/os/shared/discovery_state.rs:160-167; native/src/share/os/shared/discovery_state.rs:456-486; native/src/app/core/share_discovery_ui.rs:207-252; native/src/app/core/share_discovery_ui.rs:416-434; native/src/share/core/discovery_signal_types.rs:93-94; native/src/share/core/discovery_pake.rs:416-421; native/src/share/core/discovery_signal_commands.rs:183-199; native/src/share/core/discovery_signal_state.rs:23-24; native/src/share/core/discovery_signal_state.rs:304-317; native/src/share/core/discovery_signal_dispatch.rs:345-366; native/src/share/core/discovery_exchange_port_impl.rs:189-243; native/src/share/core/direct_reciprocal.rs:227-229; share-server/src/discovery.rs:145-168; share-server/src/discovery_state.rs:26-27; share-server/src/discovery_state.rs:131-141; README.md:378-387; docs/SHARE_SERVER.md:47-51

**Beschreibung.** The OPAQUE/Argon2id/ChaCha20-Poly1305 construction itself is sound. The problem is the PIN policy around it:
- The PIN is accepted byte-for-byte with no minimum length ('Empty input and "0" are ordinary values'). The core checks only the maximum length.
- The desktop GUI's DiscoveryPinDraft defaults to an empty string. The 'Suchbar machen' button is enabled when `pin_bytes <= MAX`. An empty PIN only produces a yellow hint, not an explicit opt-out.
- The CLI only warns. Android requires a non-empty PIN but accepts weak ones such as "1".
- The share-server lists every offer to every other connected client (filter is only `owner_id != client_id`).
- The only throttle is 12 pairing starts per offer per 60 s. There is no cap on total failures, no offer duration limit (only `duration_secs > 0`; the GUI DragValue has no maximum), and the offer is not stopped after the first successful pairing.
- A connector with a wrong PIN aborts at KE2, so the publisher only sees 'cancelled' exchanges and gets no guessing signal.
- Success needs no confirmation on the publisher. For a Direct offer the publisher persists the stranger as an Accepted contact and grant (PairingOrigin::UserPairing, which also clears any removal record) and sends its own Direct code. For a Room offer it sends the room id and 32-byte room secret to any PIN holder.

**Fehlerszenario.** 1. A user clicks 'Suchbar machen' for this device with the defaults: 5 minutes, empty PIN.
2. Any other client of the same Share server polls ListDiscoveries, sees the alias, and runs StartPairing with PIN "". KE3 verifies.
3. The publisher calls persist_direct(stranger, UserPairing), creating an Accepted contact and grant, and sends the publisher bundle with its Direct code.
Result: the stranger has mutual Direct access to the publisher's default exports (whole home, read/write).

With a Room offer, the stranger gets the room secret and therefore access to every member's room exports. With a 4-digit PIN and a long offer duration, 12 guesses per minute gives about 720 guesses per hour, and the offer stays open after the legitimate partner has paired. An attacker can also publish an offer with the same alias and the same common PIN to phish connectors.

**Richtung.** Make the secure option the default:
- Generate a random PIN (6 or more digits, or a passphrase) by default. Reject empty or low-entropy PINs in the core unless an explicit 'unsicher' opt-in flag is set.
- Make offers single-use: stop after the first completed exchange.
- Cap the offer duration.
- On the publisher, count aborted and cancelled exchanges after KE2 and close the offer after N of them.
- Require local confirmation on the publisher, showing the connector's name and fingerprint, before persisting a Direct grant or releasing a room secret.
- Optionally scope listings, for example by a short offer code.

**Gegenprüfung.** All facts check out. The desktop draft PIN defaults to an empty string (discovery_state.rs:165-167, 456-457). The publish button is enabled when `!pending && duration_secs > 0 && pin_bytes <= MAX` (share_discovery_ui.rs:226, 241). An empty PIN only triggers a warning (pin_guidance at :429-433). The duration DragValue has no range (:210-212), and prepare_offer checks only `duration_secs == 0` and the maximum PIN length (discovery_signal_commands.rs:191-199). Neither the PAKE (discovery_pake.rs:416-421) nor the type (discovery_signal_types.rs:93-94) has a minimum. The CLI only warns (cli/share/discoverable.rs:160-163). Android requires a non-empty PIN but only flags weak ones (ShareDialogs.kt:180, 220-225). The server lists every other client's offers (share-server discovery.rs:152-157). The only throttles are 12 starts per 60 s plus concurrency caps (server discovery_state.rs:19, 26-27, 131-141; client discovery_signal_state.rs:23-24, 304-317), with no cap on total failures. Completion emits only ExchangeCompleted and keeps the offer running (discovery_signal_dispatch.rs:345-366; the offer book ignores completion at discovery_offer_book.rs:56-59). Without confirmation, the publisher persists the connector as UserPairing (discovery_exchange_port_impl.rs:196-199), which also readmits removed peers (direct_reciprocal.rs:227-229), or hands out the room material (:213-242). The design is documented and intentional ('bewusst keine Mindestlaenge', README.md:380-387; SHARE_SERVER.md:47-51, 62-69), but it is insecure by default, contrary to the requirement. Critical is justified for the shipped defaults: one click on 'Suchbar machen' with an empty PIN lets any client of the same unauthenticated server get a mutual Direct relation, and with #2's default home export that means read/write access to the home folder. Standalone, without #2's exports, it would be high.

## S21 Pairing or joining a room exposes the whole home folder read/write/delete by default, and Direct pairing automatically grants the other side access

- Schwere: critical · Urteil: confirmed · Kategorie: security · Plattformen: 
- Stellen: native/src/share/core/profile_persistence.rs:63-71; native/src/share/core/profile_persistence.rs:219-232; native/src/share/os/shared/profile_operations.rs:130-139; native/src/share/os/shared/identity_store.rs:134-148; native/src/mobile/core/config.rs:84-98; android/app/src/main/java/app/smartexplorer/android/core/InitConfig.kt:38; native/src/share/core/fs.rs:23-27; native/src/share/core/fs.rs:310-343; native/src/share/core/server_fs.rs:235-255; native/src/share/core/session.rs:185-219; native/src/share/core/configuration_runtime.rs:136-181; native/src/share/core/node_sessions.rs:40-130; native/src/share/core/direct_reciprocal_transport.rs:63-139; native/src/share/os/shared/direct_repair_store_adapter.rs:120-123; native/src/share/core/direct_reciprocal.rs:339-363; native/src/support_dirs.rs:72-80; native/src/creds/os/linux_os.rs:12-14; native/src/share/core/service.rs:321-333

**Beschreibung.** On first load (profile file missing), default_direct_exports becomes [Home = the default home]. That is %USERPROFILE% or $HOME on desktop, and on Android the primary shared volume (/storage/emulated/0). ShareExportConfig has no read-only mode: an authorized principal can read, PutBatch, RemoveFile and RemoveDir (recursive) anywhere under the root. Only root containment is enforced; dotfiles, the app data folder and the Linux secret store are not filtered.

Direct relations are made mutual without asking:
1. When B adds A's code, A auto-accepts.
2. B's configuration runtime schedules a reciprocal repair for every accepted contact.
3. On B, apply_reciprocal_direct_peer pushes an Accepted DirectGrant for A.

PIN pairing persists a grant on both sides the same way. Every room joined by code or PIN copies default_direct_exports into room.exports. Everyone who holds the room secret then gets the same access.

**Fehlerszenario.** On a fresh install, user B adds a Direct code that someone posted, in order to browse that device. Within seconds:
1. A auto-accepts B.
2. B's repair worker initiates the repair; B persists A as a contact and creates an Accepted grant for A.
3. A can now list, read, write and recursively delete anywhere under B's home.

On Linux this includes ~/.local/share/smart_explorer/secrets-v1, the plaintext Iroh secret key, Direct secret and every contact and room secret. Stealing these lets A impersonate B to all of B's peers and rooms. It also includes ~/.ssh, and write access to ~/.bashrc or ~/.config/autostart, which leads to code execution at the next login. On Windows the same applies to %USERPROFILE% including the Startup folder. On Android it covers all shared storage. Joining a public room code has the same effect toward every member.

**Richtung.** - Default exports to empty (deny-all) and make the user choose roots explicitly.
- Add a read-only flag to exports, with write access opt-in.
- Do not grant access back automatically. Make reciprocity an explicit choice when adding a code, pairing or joining a room.
- Start rooms with empty exports.
- Always exclude the app data folder, the credential store and known autostart and credential paths from exports.

**Gegenprüfung.** Confirmed. On first load, a missing profile seeds default_direct_exports with Home (profile_persistence.rs:63-71). Home is USERPROFILE/HOME on desktop (identity_store.rs:138-148) and the primary shared volume on Android (InitConfig.kt:38, mobile config.rs:87-98). ShareExportConfig only has roots and include_connections, so there is no read-only mode (fs.rs:23-27). Path resolution enforces only containment under the root (fs.rs:310-343), and RemoveFile/RemoveDir (recursive) are served to any authorized stream (server_fs.rs:235-255). Direct session authorization returns default_direct_exports (session.rs:218), and direct_online defaults to true (service.rs:331). Mutuality happens without a prompt. A's auto-accept is #0's path. B's runtime then schedules a repair for each auto_connect Accepted contact with a current presence (configuration_runtime.rs:144-180). The outgoing repair (node_sessions.rs:40-130 → run_outgoing at direct_reciprocal_transport.rs:63-139) persists A through persist_direct(AutomaticRepair) (direct_repair_store_adapter.rs:120-123), and that pushes an Accepted DirectGrant for A (direct_reciprocal.rs:351-362). Rooms joined by code or PIN copy default_direct_exports (profile_persistence.rs:231, profile_operations.rs:138; the PIN path goes through discovery_relation_store_adapter.rs:72-80). On Linux the secret store is $XDG_DATA_HOME or ~/.local/share/smart_explorer/secrets-v1 (support_dirs.rs:72-80, linux_os.rs:12-14), which lies under the exported Home. Nothing excludes the app data directory or dotfiles. Mutual pairing is documented as intent (SHARE_SERVER.md:62-69, 287-293; README.md:409-415). It still makes every relation over-privileged by default, including grants the user never approved, so critical is justified.

## S22 Room members cannot really be revoked: blocking is keyed on a device_id the member chooses, the room secret cannot be rotated, and members' identities are not authenticated

- Schwere: high · Urteil: confirmed · Kategorie: security · Plattformen: 
- Stellen: native/src/share/core/session.rs:220-253; native/src/share/core/signal_auth.rs:279-320; native/src/daemon/os/shared/ipc_host_events.rs:445-484; native/src/share/os/shared/profile_edits.rs:99-106; native/src/share/core/room_relation.rs:7-43; native/src/share/core/profiles.rs:151-157; share-server/src/state.rs:154-166

**Beschreibung.** Room membership means holding the static 32-byte room secret. Any presence with a valid HMAC is accepted:
- verify_room_presence checks only the HMAC, expiry and nonce.
- upsert_room_member adds an unknown device_id as a new member with blocked:false.
- Session authorization looks the member up by hello.device_id and !blocked, then checks the HMAC.

'Blocking' only sets a flag on that one device_id entry. There is no room-secret rotation or re-key API. Removing the room locally only deletes our own copy.

Member identity is not authenticated either. Nothing checks that public_key matches node_id or that fingerprint matches public_key at ingestion. For sessions, only node_id is bound to TLS. So any member can:
- claim another device's public_key and fingerprint for its own node;
- take over the device_id of an offline member;
- send a presence with an existing member's device_id but a different key, which sets that member to IdentityConflict and resets their Exec grant (repeatable).

**Fehlerszenario.** 1. A blocks member M. M still holds the room code.
2. M restarts with a new device_id (edit share_identity.json, or identity repair) and sends JoinRoom with a presence HMAC'd with the room secret.
3. A's upsert_room_member finds no entry for that device_id and pushes a new unblocked member.
4. authorize_state accepts M's session (node equals TLS, HMAC valid).
Result: M has access to A's room exports again, which by default is A's whole home. The only remedy is to create a new room and redistribute codes to everyone.

Separately, M repeatedly publishes presences with victim V's device_id and M's key. V shows IdentityConflict at every member, and V's Exec grants are reset each time.

**Richtung.** - Add a room re-key operation: a new secret distributed only to non-blocked members over authenticated Iroh sessions.
- Record blocks by public key and node id and refuse new device_ids that present a blocked key.
- Bind member identity: require public_key == node_id and fingerprint == fp(public_key) at ingestion, and sign presences with the member's Iroh key.
- Optionally hold new members as pending until an existing member approves them.

**Gegenprüfung.** Core claims hold. A block is only a per-device_id flag (profile_edits.rs:99-105). Every enforcement point looks up the member entry by device_id: session.rs:229, peer_endpoint_source.rs:156, exec_auth/exec_grant_runtime. verify_room_presence checks only expiry, kind, relation, nonce and the HMAC (signal_auth.rs:284-319). upsert_room_member pushes an unknown device_id as `blocked: false` (ipc_host_events.rs:469-482). No room-secret rotation or re-key API exists anywhere: there is no rotate/rekey symbol, and the room code is a fixed id plus secret (profiles.rs:151-157). So a blocked member who restarts with a new device_id is admitted again. Nothing checks public_key == node_id or fingerprint == fp(public_key) on room presences. Session auth only compares the stored member key and node with the hello and the fingerprint of the stored key (session.rs:231-236). A member can therefore present a victim's public_key and fingerprint under its own node and a new device_id. A presence that reuses an existing device_id with a different key or node sets IdentityConflict and resets that member's Exec grant on every attempt (ipc_host_events.rs:451-458). One sub-claim is wrong: taking over the device_id of an offline member is blocked on the client by the same key/node check. The entry is not overwritten, unless a legacy entry has an empty node_id. Server-roster displacement is a separate issue (#6). Severity stays high: re-access after an explicit block, with no remedy other than creating a new room.

## S23 After a Direct code rotation or identity repair, every former peer is treated as explicitly ignored and cannot re-pair

- Schwere: high · Urteil: confirmed · Kategorie: bug · Plattformen: 
- Stellen: native/src/share/core/legacy_direct_request_reconciliation.rs:76-116; native/src/share/core/legacy_direct_request.rs:200-206; native/src/daemon/os/shared/ipc_host_direct_events.rs:221-237; native/src/share/core/direct_reciprocal.rs:273-281; native/src/share/core/direct_reciprocal.rs:372-407; native/src/share/core/legacy_direct_request_mutations.rs:378-382; native/src/share/core/direct_lifecycle.rs:279-289; native/src/share/os/shared/identity_store.rs:97-132; native/src/share/os/shared/profile_store.rs:188-194; native/src/share/core/identity.rs:206-237

**Beschreibung.** invalidate_direct_grants_after_identity_rotation calls invalidate_all_direct_grants, which sets every grant to DirectGrantState::Ignored. That is the same state an explicit user 'ignore' produces. It runs after regenerate_direct_code and automatically during identity repair or on load when cleanup is pending. Afterwards:
- direct_auto_accept_denied sees an exact ignored grant for every former peer, so new requests that carry the new code are auto-Rejected at revision 1. Rejected allows no further transition, so the user cannot accept them later.
- PIN pairing fails with PolicyDenied(GrantIgnored).
- Reciprocal repair from our side conflicts, because the peer's contact still holds the old lookup_id.
- Legacy requests are Rejected through ExistingGrant.

There is no 'allow again' path for an ignored grant: set_direct_grant_persisted is never called. Only 'Eintrag loeschen' (which writes a removal), then 'Erneut zulassen', then the peer resending works.

**Fehlerszenario.** 1. The user rotates the Direct code because it leaked, and gives the new code to trusted device P.
2. P adds it and its request arrives. ignored_grant matches P's exact identity, so the decision is Rejected; P's contact becomes Ignored.
3. The user then tries PIN pairing with P. It fails with 'Direct grant is explicitly ignored'.

The same happens to every peer after an automatic IdentityReplaced repair, for example when the Windows credential or Linux secrets-v1 entry for the Iroh key was lost.

**Richtung.** - Record rotation-invalidated grants with a distinct state or source (for example Suspended or AuthorizationLost) that is not treated as a user denial. Let a request authenticated with the new code, or a deliberate PIN pairing, re-activate them.
- Add an explicit 'allow again' action for ignored grants.
- Optionally push the new code to accepted peers over an authenticated repair.

**Gegenprüfung.** Confirmed. Rotation saves pending_cleanup DirectCodeRotated (identity.rs:217-218). Every identity load completes it through finish_pending_cleanup_locked → invalidate_direct_grants_after_identity_rotation (identity_store.rs:97-132). invalidate_all_direct_grants sets each Accepted or exec-enabled grant to DirectGrantState::Ignored (legacy_direct_request_reconciliation.rs:104-113), the same state an explicit ignore produces. The peer's identity has not changed, so its next tracked request carrying the new code matches ignored_grant exactly (legacy_direct_request.rs:200-206) and is auto-Rejected at revision 1 (ipc_host_direct_events.rs:225). Rejected allows no later transition (direct_lifecycle.rs:282-286). Legacy requests are Rejected through ExistingGrant (legacy_direct_request_mutations.rs:378-382). PIN pairing hits PolicyDenied(GrantIgnored) (direct_reciprocal.rs:273-281). Our own repair conflicts because the peer's contact keeps the old lookup_id (contact_matches at :410-430 via :372-407). No path sets an ignored grant back to Accepted: set_direct_grant_persisted (profile_store.rs:188) has no callers, and grep finds no UI or CLI path. Recovery needs grant deletion, which writes a removal record, then readmit or PIN pairing, then a resend, for each peer. IdentityReplaced does the same automatically after the Iroh credential is lost. Code rotation is the only real revocation tool (see #0), and an automatic repair breaks every relation, so high is appropriate.

## S24 PIN pairing grants access before the other side commits; an aborted exchange shows 'failed' although the grant is already active

- Schwere: medium · Urteil: confirmed · Kategorie: bug · Plattformen: 
- Stellen: native/src/share/core/discovery_exchange_port_impl.rs:189-212; native/src/share/core/discovery_exchange_port_impl.rs:119-158; native/src/share/core/discovery_exchange_port_impl.rs:247-264; native/src/share/core/discovery_signal_persisted.rs:36-44; native/src/share/core/discovery_signal_dispatch.rs:380-401; native/src/share/core/discovery_signal_maintenance.rs:296-324

**Beschreibung.** When the publisher receives KE3 and the connector bundle, it immediately runs persist_direct (Accepted contact and grant). It then applies the profiles live and sends the PublisherBundle containing its own Direct code. Only afterwards does it wait for ConnectorCommit. The connector likewise persists the publisher before sending its commit. If the exchange then ends without Completed (timeout after 2 minutes, cancel, disconnect, or the server dropping the last packet), handle_finished and expire_exchanges emit ExchangeFailed or Cancelled. Nothing rolls the relation back.

**Fehlerszenario.** 1. A connector that knows the PIN sends KE3 and its bundle.
2. The publisher persists the connector as Accepted and sends the PublisherBundle.
3. The connector, or a malicious server, withholds ConnectorCommit.
4. After 120 s the publisher UI shows 'Exchange fehlgeschlagen: Discovery-Austausch hat sein Zeitlimit erreicht'.
Result: the connector keeps an active grant and contact and the publisher's Direct code. The publisher's user believes nothing was paired.

**Richtung.** Stage the relation and commit it only after the counterpart's commit is verified, rolling back the staged relation on abort. Alternatively, report 'paired (unconfirmed)' with an immediate revoke option instead of 'failed'.

**Gegenprüfung.** Confirmed. The publisher persists the connector (persist_direct UserPairing) and sends PublisherBundle with its own Direct code at KE3 (discovery_exchange_port_impl.rs:189-211). discovery_signal_persisted.rs:39-44 applies the profiles live before sending. ConnectorCommit is only handled afterwards (:247-253). The connector also persists before its commit (:135-157). Without Completed, finish_exchange returns None and nothing is rolled back (:412-420). handle_finished emits ExchangeFailed or Cancelled (discovery_signal_dispatch.rs:380-400), and expire_exchanges emits 'Discovery-Austausch hat sein Zeitlimit erreicht' after DISCOVERY_EXCHANGE_TIMEOUT = 120 s (discovery_signal_maintenance.rs:296-322, discovery_signal_state.rs:22). Persist-before-commit is documented intent (SHARE_SERVER.md:62-66), but the failure report is misleading. A connector who guessed a weak PIN (#1) can withhold ConnectorCommit, so a successful pairing is shown to the publisher as a failure. Medium is appropriate.

## S25 Share-server registrations are unauthenticated: anyone can take over a device's lookup_id registration, push it out of a room roster, or receive messages addressed to its device_id

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: 
- Stellen: share-server/src/protocol.rs:106-117; share-server/src/transport.rs:120-150; share-server/src/tracked_direct.rs:206-241; share-server/src/tracked_direct.rs:378-419; share-server/src/state.rs:60-102; share-server/src/state.rs:154-166

**Beschreibung.** Hello carries device_id, public_key and fingerprint without any proof of key possession. The server ignores public_key and registers the client under the device_id it claims; duplicates are allowed.
- PublishDirect only checks that client.device_id equals presence.device_id, then overwrites state.direct[lookup_id] with the new owner. Lookup ownership is never checked.
- route_request forwards tracked requests to whoever owns the lookup_id.
- Decisions, receipts and legacy accepts are routed to every client claiming the device_id.
- join_room replaces an existing member entry with the same device_id and removes the room from the previous client.
The content stays authenticated end to end, but availability and routing metadata do not.

**Fehlerszenario.** A removed peer knows the victim's lookup_id (from the Direct code) and loops PublishDirect{relation_id: victim lookup}. Then:
- WatchDirect and DirectAvailable serve the attacker's presence, which contacts reject as invalid HMAC, so the victim appears offline.
- Tracked requests for the victim (requester identity, device name, message) are delivered to the attacker and acknowledged as Forwarded.
Similarly, with a room_id and the victim's device_id, JoinRoom pushes the victim out of the server roster, so the victim stops receiving RoomJoined/RoomLeft until it re-joins.

**Richtung.** - Authenticate registrations with a server nonce signed by the Iroh key, and bind device_id to that key.
- Bind lookup_id and room-member slots to the first authenticated key and refuse overwrites from other keys.
- Route decisions and receipts by authenticated node id rather than a claimed device_id.

**Gegenprüfung.** Confirmed. Hello's public_key and fingerprint are discarded (transport.rs:120-128), and the client registers under any valid self-claimed device_id, duplicates included (state.rs:60-101). PublishDirect only requires client.device_id == presence.device_id and then overwrites state.direct[lookup_id] without any ownership check (tracked_direct.rs:206-231). Requests are routed to the current lookup owner (route_request → request_target, tracked_direct.rs:43-85, 378-390). Receipts, decisions and legacy accepts are routed to every client that claims the device_id (:101, :120, :139, :192, :392-419). join_room replaces the member with the same device_id and removes the room from the previous client (state.rs:154-166), so that client stops getting RoomJoined. Disconnect cleanup then emits DirectOffline for a hijacked lookup (state.rs:259-275). Contents stay end-to-end authenticated: forged receipts need the target's Iroh signature. The impact is availability and metadata leakage, so medium fits.

## S26 Presence HMAC does not cover fingerprint or device_name, yet the receiver stores both and uses the fingerprint in authorization checks

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: 
- Stellen: native/src/share/core/crypto.rs:136-153; native/src/share/core/signal_presence.rs:22-46; native/src/share/core/signal_auth.rs:226-320; native/src/daemon/os/shared/ipc_host_events.rs:460-461; native/src/share/core/session.rs:234-236; native/src/share/core/direct_reciprocal_coordinator.rs:102-108

**Beschreibung.** presence_payload signs kind, relation, device_id, public_key, node, relay, candidates, expiry and nonce, but not device_name or fingerprint. Neither verify_direct_presence nor verify_room_presence checks presence.fingerprint against fp(public_key). upsert_room_member copies presence.fingerprint and device_name into the member. Later checks use the stored fingerprint: room session authorization (fingerprint_matches(member.public_key, member.fingerprint)), the Direct repair candidate and mount pin checks (contact.expected_fingerprint == presence.fingerprint). Mod.rs describes the rendezvous server as untrusted, yet that server, or an on-path attacker on plaintext signaling, can rewrite these fields.

**Fehlerszenario.** A malicious server rewrites `fingerprint` in a member's RoomRoster or RoomJoined presence. The receiver stores it, and authorize_state then rejects that member with 'Raumgeraet hat ungueltigen Fingerprint', so the member is locked out of every receiver. The server can also rename members (device_name), for example to impersonate a known person in the room list or the legacy request inbox. For Direct, a rewritten fingerprint in DirectAvailable makes DirectRepairCandidate return IdentityConflict, so reciprocal repair is never scheduled and mounts fail with 'Pins wurden geaendert'.

**Richtung.** Derive the fingerprint locally from the authenticated public_key and never trust the wire value. Include device_name in the MAC, or better, sign presences with the device's Iroh key.

**Gegenprüfung.** Confirmed. presence_payload covers kind, relation, device_id, public_key, node, relay, candidates, expiry and nonce, but not device_name or fingerprint (crypto.rs:136-153). build_presence sends both fields unsigned (signal_presence.rs:22-46). verify_room_presence never inspects presence.fingerprint (signal_auth.rs:279-320). verify_direct_presence compares fp(presence.public_key) with the contact pin (:248) but never checks the presence.fingerprint field. upsert_room_member stores the unsigned device_name and fingerprint (ipc_host_events.rs:460-461, 470-472). Later checks use those stored fields: room session fingerprint_matches(member.public_key, member.fingerprint) (session.rs:234-236), DirectRepairCandidate expected_fingerprint vs presence.fingerprint (direct_reciprocal_coordinator.rs:102-107), and the mount pin checks (peer_endpoint_source.rs:91-110, 120-137). A malicious server, or an on-path attacker with #8's plaintext transports, can therefore lock out members or Direct repairs and rename room members and legacy-inbox requesters (legacy_direct_request.rs:301-309 takes device_name from the presence). Tracked envelopes are not affected: direct_transcript.rs:251-254 signs device_name and fingerprint. DoS is already possible for the server, but name spoofing could mislead per-member decisions such as Exec grants. Medium is fair.

## S27 Signaling falls back to unencrypted transport without warning; a bare host:port uses plaintext TCP

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: 
- Stellen: native/src/share/core/signal_connection.rs:71-96; native/src/share/core/signal_connection.rs:341-348; native/src/share/core/signal_connector.rs:37-50

**Beschreibung.** connect_one sends ws:// and wss:// to WebSocket, tcp:// to raw TCP, and any value without a scheme (for example 'server.example:51820') to plaintext TCP. http:// is rewritten to ws://. No warning or opt-in is required. The client Hello also sends LAN IPs, the device name, key and fingerprint, even though the server ignores the IPs. End-to-end integrity holds through HMAC, signatures and OPAQUE, but an on-path attacker on such a link sees everything the server sees and can act as the server. That includes device ids and names, lookup and room ids, LAN and public address candidates, discovery aliases, the social graph through watches, and the ability to cause the routing and denial-of-service effects described in other findings.

**Fehlerszenario.** A user enters the server as 'myshare.example:51820'. On café Wi-Fi, an attacker can read every presence (IP candidates, device names, room ids), enumerate discovery offers, and inject or drop signaling. They can also hijack routing or rewrite unauthenticated presence fields.

**Richtung.** Default to wss:// with TLS verification. Reject tcp://, ws:// and scheme-less addresses unless an explicit 'unverschluesselt' opt-in is set, and show a persistent warning. Stop sending LAN IPs in Hello.

**Gegenprüfung.** Confirmed. connect_one routes ws:// and wss:// to WebSocket, tcp:// to raw TCP, and a value with no scheme to plaintext TCP (signal_connection.rs:71-83). http:// becomes ws:// (:341-347). connect() tries comma-separated fallbacks in order (:59-64), so the documented mixed list `wss://..., host:51820` (SHARE_SERVER.md:28-31) lets an on-path attacker block TLS and force plaintext. The settings hint shows host:port first and only claims end-to-end encryption, with no plaintext warning (menus_settings.rs:14-23); grep finds no warning in the app or CLI. Hello sends LAN IPs, name, key and fingerprint (signal_connector.rs:37-50), and the server ignores them (transport.rs:120-128). File data stays QUIC-encrypted and envelopes stay HMAC- or signature-protected. The exposure is metadata plus the routing and unsigned-field effects in #6, #7 and #11, so medium.

## S28 Anyone holding the Direct code can fill the 24-entry request ledger and the 24-entry legacy inbox with auto-accepted entries that are never pruned, blocking all new requests

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: 
- Stellen: native/src/share/core/direct_ledger.rs:17; native/src/share/core/direct_ledger_retention.rs:27-53; native/src/share/core/direct_ledger_mutations.rs:50; native/src/share/core/direct_ledger_mutations.rs:80; native/src/share/core/legacy_direct_request.rs:11; native/src/share/core/legacy_direct_request_mutations.rs:87-91; native/src/daemon/os/shared/ipc_host_direct_events.rs:237; native/src/share/core/removed_direct_peers.rs:19; native/src/share/core/removed_direct_peers.rs:89-93; native/src/share/core/removed_direct_peers.rs:172-195

**Beschreibung.** Incoming and outgoing tracked requests share one ledger of 24 entries. can_prune never evicts Pending or Accepted entries. Every request from a new identity carrying the Direct code is auto-Accepted, and each adds an Accepted grant. Signing keys and device_ids cost an attacker nothing. The legacy inbox has the same 24-entry limit and auto-accepts in the same way. Clearing them requires the user to revoke and then delete each entry by hand.

**Fehlerszenario.** A former peer submits 24 tracked requests from 24 fresh keys and device_ids. All are auto-Accepted, filling the ledger with unprunable entries and adding 24 junk grants with access. After that, ensure_direct_request_capacity returns LedgerFull for every legitimate incoming request and for the user's own new outgoing requests (adding any Direct code fails) until the user cleans up manually. The legacy inbox reports 'legacy request inbox is full' the same way.

**Richtung.** - Give incoming requests from unknown identities their own small quota and rate limit, separate from outgoing requests.
- Make settled auto-accepted history prunable (the grant holds the authorization, not the ledger entry).
- Alert the user when new identities are being auto-accepted in bulk.

**Gegenprüfung.** The mechanism is confirmed. Both directions share MAX_DIRECT_REQUEST_ENTRIES = 24 (direct_ledger.rs:17; capacity checks at direct_ledger_mutations.rs:50 and :80). can_prune never evicts Pending or Accepted entries (direct_ledger_retention.rs:51). Each fresh identity that carries the code is auto-Accepted with a grant (ipc_host_direct_events.rs:237). LedgerFull becomes a Permanent apply error (ipc_host_direct_events.rs:331-341), so legitimate incoming requests fail. The user's own add-peer also fails (direct_actions.rs:84-87). The legacy inbox caps at 24 with no auto-pruning (legacy_direct_request_mutations.rs:87-91). Correction to the remediation: no revoke-then-delete per entry is needed. 'Eintrag loeschen' (delete_direct_grant, removed_direct_peers.rs:172-195) removes a device's grant and all its tracked and legacy requests in one step, but that is still one manual action per junk identity. Each deletion also writes a removal record into a 64-entry ledger that drops the oldest records (removed_direct_peers.rs:19, 89-93), so a mass cleanup can evict older, legitimate removal records. The attacker needs the Direct code, which already grants access, so the extra harm is DoS: medium.

## S29 Removing a Direct contact whose remote_device_id is not yet known leaves that device's active grant in place and records no removal

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: 
- Stellen: native/src/share/core/removed_direct_peers.rs:104-167; native/src/share/os/shared/removal.rs:55-62; native/src/daemon/os/shared/ipc_host_events.rs:104-108; native/src/share/core/direct_reciprocal.rs:290-321; native/src/share/core/direct_ledger_projection.rs:53-54

**Beschreibung.** forget_direct_peer derives the device to deny only from contact.remote_device_id. contact_remote_identity returns None when that field is empty. This is the case for contacts added by code whose request has not been answered yet, and for legacy contacts. Grants are deleted only by that device_id. A grant created independently, for example because the peer added our code and we auto-accepted it, is not matched against the contact's pinned expected_node_id or fingerprint, so it survives. The DirectAvailable handler also overwrites remote_device_id from any presence that passes verification. Any other holder of the peer's Direct secret can forge such a presence, which changes which device id a later removal targets.

**Fehlerszenario.** 1. A and B have each added the other's code. A's request to B is still pending because B is offline, so A's contact for B has remote_device_id None.
2. B's earlier request to A was auto-accepted, so A holds an Accepted grant for B.
3. A clicks 'Entfernen' on B. The headline says '{name} entfernt'.
Result: B's grant stays Accepted, no removal record is written, and B keeps access to A's exports.

**Richtung.** When removing a contact, also delete and deny grants whose node_id or fingerprint matches the contact's expected_node_id or expected_fingerprint. Record the removal by key as well as device_id. Do not overwrite pinned remote_device_id or public key from presence.

**Gegenprüfung.** Code facts are confirmed. contact_remote_identity returns None when remote_device_id is empty (removed_direct_peers.rs:106-110). forget_direct_peer then removes no grant and writes no removal record (:145-165). The grant is never matched against the contact's expected_node_id or fingerprint, and the headline is a plain '{name} entfernt' (removal.rs:61). DirectAvailable overwrites remote_device_id and remote_public_key from any verified presence (ipc_host_events.rs:107-108). The verification pins only fp(public_key) and node (signal_auth.rs:248-253), not device_id, so another holder of the peer's code can plant a fake device_id. The scenario is narrow, though. remote_device_id is set by the first verified DirectAvailable (ipc_host_events.rs:107), by any decision (direct_ledger_projection.rs:53), and by repair or PIN pairing (direct_reciprocal.rs:290-292, 314-321). The unremoved grant requires the peer to have been auto-accepted earlier with no completed reciprocal repair, plus the contact added later while the peer stays offline, or a deliberate forgery by a third code holder. The surviving grant stays visible under 'Autorisierte Geraete'. Low is more appropriate than medium.

## S30 Direct presences and legacy accept/reject messages are authenticated only by the target's device-wide Direct secret, so any other holder of that code can forge them to the target's other peers

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: 
- Stellen: native/src/share/core/signal_auth.rs:52-74; native/src/share/core/signal_auth.rs:159-277; native/src/daemon/os/shared/ipc_host_events.rs:86-118; native/src/daemon/os/shared/ipc_host_events.rs:152-201; share-server/src/tracked_direct.rs:174-204

**Beschreibung.** verify_direct_presence and verify_direct_access_accepted_using check only an HMAC with the contact secret (the target's global Direct secret, which every peer of the target holds), plus the fingerprint and node pins that the attacker can copy. Nothing is signed with the target's Iroh key. Legacy DirectAccessAccepted is processed even when tracked_direct_v1 is negotiated. The server forwards it to any device_id the sender names, and the daemon sets the contact to Ignored with a message the attacker chooses, or overwrites remote_device_id and public key from the forged presence.

**Fehlerszenario.** M is another peer of T and knows V's device_id. M sends DirectAccessAccepted{lookup: T, requester_device_id: V, accepted:false, msg:'...', presence: HMAC with T's secret, claiming T's key and node}. V's contact for T flips to Ignored with M's text and stops connecting. Alternatively, M forges a DirectAvailable for T's lookup with a different device_id. V's contact pins remote_device_id to the forged value, which breaks session proofs and makes V's later 'remove T' miss T's real grant.

**Richtung.** Require Iroh-key signatures on Direct presences and decisions, at least verifying against the pinned node key. Drop the legacy decision path when tracked_direct is negotiated. Never overwrite remote_device_id or public key from presence once they are pinned.

**Gegenprüfung.** Confirmed. verify_direct_access_accepted_using checks only that requester_device_id is the local id, freshness, kind/relation, fp(presence.public_key) against the contact pin, the node pin, the nonce, and an HMAC with the contact secret (signal_auth.rs:159-224). That secret is the target's device-wide Direct secret, which every peer of the target holds (identity.rs:186-194, discovery_signal_port.rs:26-43). Nothing is signed with the target's Iroh key. The message is handled with no tracked_direct_v1 gating (signal_auth.rs:52-74). The daemon flips the contact to Ignored with the sender's text (ipc_host_events.rs:181-200), or on accepted:true overwrites remote_device_id and keys (:184-191). The server forwards legacy decisions to any named device_id without checking lookup ownership (share-server main.rs:164-178; tracked_direct.rs:174-204; direct_validation.rs:142-154). A forged Ignored state persists in direct_auto_accept_denied's ignored_contact check (legacy_direct_request.rs:207-213) and blocks repair with ContactIgnored (direct_reciprocal.rs:264-272) until V retries manually. The attacker must be another peer of the target, and the effect is DoS and relation-state corruption, so medium.

## S31 Windows: the Share identity roams with the user profile (Roaming AppData plus Credential Manager with ENTERPRISE persistence), so several domain PCs can end up with the same identity

- Schwere: low · Urteil: confirmed · Kategorie: platform · Plattformen: 
- Stellen: native/src/support_dirs.rs:64-70; native/src/creds/os/windows.rs:44-48; native/src/share/os/shared/identity_store.rs:286-301; native/src/share/os/shared/identity_store.rs:315-317

**Beschreibung.** share_identity.json and share_profiles.json live in %APPDATA% (Roaming). The Iroh secret and Direct secret are stored through keyring 3.6.3 set_password, which writes CRED_PERSIST_ENTERPRISE credentials (keyring-3.6.3/src/windows.rs:246). These credentials roam with roaming user profiles in a domain. Two machines that use the same roaming profile load the same device_id, Iroh key and Direct code.

**Fehlerszenario.** In a domain with roaming profiles, a user logs on to PC1 and PC2. Both publish the same lookup_id and Iroh node id, and the server and relay switch between them. Grants that peers gave 'this device' authorize both PCs. Removing or blocking one removes the other. Profile changes on one machine overwrite the other's at logoff.

**Richtung.** Store the identity and profiles in %LOCALAPPDATA%. Write the identity secrets with CRED_PERSIST_LOCAL_MACHINE (direct CredWriteW, or another store) or with DPAPI-protected local files. Detect a cloned identity (the same node id announced from different hosts) and offer identity repair.

**Gegenprüfung.** Confirmed. On Windows, app data is %APPDATA%, the roaming folder (support_dirs.rs:64-70), which holds share_identity.json (identity_store.rs:315-317). Secrets go through keyring set_password (creds/os/windows.rs:44-48). The locked keyring is 3.6.3 (Cargo.lock:3722-3724), and its windows.rs:246 uses `CRED_PERSIST_ENTERPRISE`, which roams with roaming user profiles. No machine binding exists: COMPUTERNAME is used only for default names. Two domain PCs with one roaming profile therefore share device_id, Iroh key and Direct code. I would lower the severity to low. It needs an AD roaming-profile setup, both machines belong to the same user account, and the result is identity collision and flaky connectivity (lookup and relay flapping, shared revocation), not access by another person.

## S32 Clock skew between devices makes tracked envelopes fail with permanent errors until the receiver's clock catches up

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: 
- Stellen: native/src/share/core/direct_ledger.rs:181-186; native/src/share/core/direct_lifecycle.rs:379-393; native/src/share/core/direct_lifecycle.rs:343-350; native/src/share/core/direct_ledger_mutations.rs:160-168; native/src/share/core/direct_protocol.rs:402-417

**Beschreibung.** verify_at allows the sender's clock to be up to 5 minutes ahead. However, recording an incoming request applies Delivery::Received at the local received_at, which must not be earlier than the sender's created_at. Recording a decision applies DecisionDelivery::Received at the local observed_at, which must not be earlier than the target's decided_at. Request receipts use max(received_at, changed_at); decisions do not. Each mismatch fails the whole apply group as a Permanent error. Retries (backoff of at most 60 s) succeed only after the local clock passes the remote timestamp.

**Fehlerszenario.** The requester's clock is 90 s ahead of the target's. The target rejects the request (InvalidTimestamp) on every retry for about 90 s and shows 'Tracked-Direct' errors before it finally records the request. The mirror case affects decisions when the target's clock is ahead.

**Richtung.** Clamp the local observation timestamps to max(local, remote) as the receipt path already does, or record remote and local times separately. Do not classify these timing conflicts as permanent.

**Gegenprüfung.** Confirmed. verify_at accepts created_at up to now+300 (direct_protocol.rs:402-417). Decisions get the same window through direct_messages.rs:158-165. DirectRequestEntry::incoming then applies Received at the local received_at (direct_ledger.rs:165-187), and validate_delivery_time requires at >= created_at (direct_lifecycle.rs:379-384). On the requester side, record_direct_decision applies DecisionDelivery Received at observed_at without max() (direct_ledger_mutations.rs:160-168), while request receipts do use max() (:107-114). advance_decision_delivery requires at >= decision.changed_at (direct_lifecycle.rs:343-350). The resulting DirectLedgerError becomes a Permanent ApplyError (ipc_host_direct_events.rs:331-341). The peer's outbox retries with backoff capped at 60 s (tracked_signal_outbox.rs:8, 112) until the local clock catches up. That gives a delay of up to the skew (at most 300 s) with transient error messages, so low is right.

## S33 Secrets are stored unencrypted at rest on Linux and Android, and the Linux identity and profile files are readable by other local users

- Schwere: low · Urteil: partially_confirmed · Kategorie: security · Plattformen: 
- Stellen: native/src/creds/os/linux_os.rs:1-14; native/src/creds/os/linux_file_store.rs:9-10; native/src/share/os/shared/identity_store.rs:231-257; native/src/share/os/shared/profile_store.rs:360-392; native/src/support_dirs.rs:89-93; README.md:523-529; android/app/src/main/AndroidManifest.xml:51-52

**Beschreibung.** Key generation is sound: getrandom provides 32-byte Iroh and Direct secrets, a 96-bit lookup id, a UUIDv4 device id, and a 32-byte room secret.

Storage is weaker:
- Linux: secrets are plaintext records in ~/.local/share/smart_explorer/secrets-v1 (0700/0600). Every process running as the user, and every home backup, gets the Iroh private key and all relation secrets. The default Home export (see the default-export finding) also exposes them to peers.
- Android: secrets are plaintext in app-private filesDir. Backups are correctly disabled (allowBackup=false and data_extraction_rules), but the key is not wrapped with Android Keystore.
- share_identity.json and share_profiles.json are created with the process umask (typically 0644) inside a 0755 data folder. Other local Linux users can read lookup ids, room ids, device ids and peer IP candidates. That is enough for the routing and roster attacks described in the server-registration finding.

**Fehlerszenario.** On a shared Linux workstation, another local user reads ~/.local/share/smart_explorer/share_profiles.json, learns the victim's room ids and device ids, and pushes the victim out of room rosters on the server. A backup of $HOME restored on another machine clones the Share identity.

**Richtung.** Create the app data folder with mode 0700 and the identity and profile files with 0600 on Unix. Use Secret Service or KWallet when available, or seal the file store with an OS-held key. Wrap the identity secrets with an Android Keystore key. Document that home backups contain the identity.

**Gegenprüfung.** Code facts hold. Linux and Android (the same `cfg(not(windows))` store, creds/mod.rs:5-10) keep secrets as plaintext records in 0700/0600 files (linux_os.rs:1-14, linux_file_store.rs:9-10). share_identity.json and share_profiles.json are created with OpenOptions without a mode, so the umask applies (identity_store.rs:248-251, profile_store.rs:372-376), inside a create_dir_all directory (support_dirs.rs:89-93). Android backups are disabled (AndroidManifest.xml:51-52), and Keystore wrapping is absent. Two parts are overstated. Plaintext-at-rest on Linux is a documented design choice: the README says it protects against other users but not against the same user or offline disk access (README.md:523-529). The JSON files being readable by other local users depends on $HOME traversal permissions. Many current distributions create homes as 0750 or 0700, which blocks access, so this is not generally true. Android filesDir is sandboxed per app. The serious exposure of secrets-v1 to peers comes from #2, not this finding. Low.

## S34 ShareIdentity and PeerEndpoint derive Debug while holding raw 32-byte secrets

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: 
- Stellen: native/src/share/core/identity.rs:29-39; native/src/share/core/types.rs:213-220; native/src/share/core/identity_repair.rs:3-8

**Beschreibung.** Most secret-bearing types in the module use redacting Debug implementations. ShareIdentity is a public API type and uses #[derive(Clone, Debug)] with `direct_secret: [u8; 32]`. PeerEndpoint also derives Debug with `relation_secret: Vec<u8>` (a contact or room secret). The Iroh SecretKey redacts itself, but these arrays would print in full in any {:?}, panic or assert message. No production {:?} use was found, so this is a latent leak.

**Fehlerszenario.** A future error message such as `format!("{endpoint:?}")` or `dbg!(identity)`, or a panic inside code that formats these structs, writes the device's Direct secret or a room secret into logs or crash reports.

**Richtung.** Write manual redacting Debug implementations, or wrap the secrets in a Zeroizing newtype whose Debug is redacted.

**Gegenprüfung.** Confirmed. ShareIdentity derives Debug with `pub(crate) direct_secret: [u8; 32]` (identity.rs:29-39), and PeerEndpoint derives Debug with `relation_secret: Vec<u8>` (types.rs:213-220). Comparable types redact their secrets, for example ShareAuthState (types.rs:474-480), DirectCode, RoomCode, DirectRelationMaterial and DirectRepairCandidate. IdentityRepair also derives Debug and embeds ShareIdentity (identity_repair.rs:3-8), which is one more vector. A grep found no production {:?} on these types, so the leak is latent. Low.

## S35 The identity transaction lock waits indefinitely, with no timeout or diagnostic

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: 
- Stellen: native/src/share/os/windows/identity_lock.rs:21-37; native/src/share/os/linux_os/identity_lock.rs:104-114; native/src/share/os/shared/identity_store.rs:303-313

**Beschreibung.** On Windows the lock opens with share mode 0 and retries every 25 ms forever on ERROR_SHARING_VIOLATION. A virus scanner, indexer or backup agent that holds the file open, or a hung process, keeps every identity operation spinning. On Linux and Android a blocking flock(LOCK_EX) has no timeout. Every identity load and mutation needs this lock, including direct decisions, rotation, repair and load_or_create.

**Fehlerszenario.** A security agent keeps identity-lock-v1\transaction.lock open, or a stalled GUI process holds the flock. ShareIdentity::load_or_create and decide_direct_request never return, so Share start or the decision dialog hangs without an error.

**Richtung.** Bound the wait (for example 10–30 s) and then return an error naming the lock file. On Linux use LOCK_NB with timed retries.

**Gegenprüfung.** Confirmed. On Windows the lock is opened with share_mode(0) and retried every 25 ms forever on ERROR_SHARING_VIOLATION, with no deadline (os/windows/identity_lock.rs:21-37, 40-48). On Linux and Android a blocking flock(LOCK_EX) loops only on EINTR (os/linux_os/identity_lock.rs:104-114). Every identity load and mutation takes this lock (identity_store.rs:20-95, 303-313), including decide_direct_request (direct_actions.rs:106-121). In practice flock is released when the holder dies, so only a hung or stopped holder blocks. On Windows a scanner handle only causes transient waits. Low is right.

## S36 Share exports have no read-only mode; every accepted Direct/Room peer gets read-write, and the first-run default export is the whole home directory

- Schwere: high · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/fs.rs:17-27; native/src/share/core/types.rs:177-209 (DirectGrant/RoomMember/RoomProfile carry no filesystem mode); native/src/share/core/session.rs:185-256 (authorize returns only the export config: Direct 218, Room 252); native/src/share/core/wire.rs:412-430 + native/src/share/core/server.rs:294-295 (mutates_filesystem only selects re-admission); native/src/share/core/server_fs.rs:118-256; native/src/share/core/fs.rs:213-233 + native/src/vfs/os/shared/local.rs:167-172 (permanent deletes); native/src/share/core/profile_persistence.rs:63-71; native/src/app/core/init.rs:54-55,67; android/app/src/main/java/app/smartexplorer/android/core/InitConfig.kt:38,85 + native/src/mobile/os/shared/domains/share_state.rs:395-398 (Android default home; support_dirs.rs:113 is only a #[cfg(test)] fixture); native/src/share/core/profile_persistence.rs:231, native/src/share/os/shared/profile_operations.rs:138, native/src/share/core/discovery_relation_store.rs:394 (rooms copy the defaults); native/src/daemon/os/shared/ipc_host_events.rs:204-247,469-482 (room members auto-admitted, blocked=false)

**Beschreibung.** ShareExportConfig carries only a list of roots plus include_connections; there is no per-export or per-root read-only flag, and nothing in the wire protocol or the server dispatcher distinguishes read from write. Any path that resolves under an exported root can be written, renamed, deleted, overwritten and copied. On first run (no profile file) the loader automatically adds the user's entire home directory as the default Direct export (desktop: USERPROFILE/HOME; Android: /storage/emulated/0, the whole primary shared storage), and rooms copy that same default at creation. The user's stated goal is 'secure by default, less secure only as an explicit opt-out'; here the only available mode is full read-write to broad roots, and read-only cannot be chosen at all.

**Fehlerszenario.** A user pairs a second device as a Direct contact (or shares a room code) intending to let it browse/copy files. That peer immediately has write access to the entire home directory: it can overwrite, rename or delete any file under it (RemoveDir does a recursive delete, server_fs.rs:246-255). There is no setting to grant read-only access, so a user who wants to share files for reading only cannot, and a compromised or careless paired device can destroy data.

**Richtung.** Add an explicit per-root (or per-export/per-grant) access mode defaulting to read-only, enforced in the server dispatcher before any mutating FsRequest (reject Write/WriteNew/Mkdir/CreateDir/Rename*/Promote*/Remove*/CopyFile/PutBatch/DiscardStage when the resolved root is read-only), surfaced in Capabilities so clients show it, and make the first-run default a read-only share (or none) so write access is an explicit opt-in.

**Gegenprüfung.** The core claim holds. ShareExportConfig has only roots and include_connections, and SharedRoot is just {label,path} (fs.rs:17-27). DirectGrant, RoomMember and RoomProfile have no filesystem access mode either (types.rs:177-209). session.authorize_state returns only the export config: default_direct_exports for an accepted Direct grant (session.rs:192-218) and room.exports for a room member (220-252). FsRequest::mutates_filesystem (wire.rs:412-430) is used only to decide lease re-admission (server.rs:294-295). The dispatcher serves Write, WriteNew, MkdirAll, CreateDir, Rename, Promote*, CopyFile, RemoveFile and RemoveDir for any resolvable path with no policy gate (server_fs.rs:118-256). RemoveDir is a recursive delete (fs.rs:213-233), and local deletes are permanent: std::fs::remove_file/remove_dir with no trash (vfs/os/shared/local.rs:167-172; linux local_platform.rs:37-39; windows local_platform.rs:118-126). rg finds no read-only/readonly gate anywhere in native/src/share; the Android readOnly flags (FilesScreen/FileList) only steer the client UI. On a missing profile the loader adds a 'Home' root (profile_persistence.rs:63-71). That root is the desktop home (init.rs:54-55,67) or, on Android, the primary volume or Environment.getExternalStorageDirectory() (InitConfig.kt:38,85 via share_state.rs:395-398), and the app holds MANAGE_EXTERNAL_STORAGE (AndroidManifest.xml:18). One cited location is wrong: support_dirs.rs:113 is a unit-test fixture, not the production source. Rooms copy the default exports on every creation path (profile_persistence.rs:231; profile_operations.rs:138; discovery_relation_store.rs:394). Room members are added automatically and unblocked from verified roster/join events (ipc_host_events.rs:204-247,469-482), so every holder of a room code gets read-write home by default. Direct peers do need an explicit Accepted grant (session.rs:192-202), which limits exposure but does not provide read-only. Severity high is justified, arguably understated: home contains autostart locations and the app's own data directory (%APPDATA%/smart_explorer or ~/.local/share/smart_explorer, support_dirs.rs:63-90). Default write access is therefore an indirect code-execution path that bypasses the separately gated Exec feature (server.rs:340-343).

## S37 No per-principal fairness on the 32-slot metadata/control pool: one authorized peer can stall browsing, stat, walk, snapshot, rename and delete for all peers

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/blocking.rs:16-23,41-55; native/src/share/core/walk.rs:29-32,44-102; native/src/share/core/storage_snapshot.rs:26-32,89-130; native/src/share/core/server_fs.rs:89-98,140-256,347-380; native/src/share/core/server_capabilities.rs:36; native/src/share/core/server.rs:154-189,274; native/src/share/core/keepalive.rs:8,112

**Beschreibung.** Transfers have per-principal admission slots (server_admission.rs: PRINCIPAL_TRANSFER_SLOTS=60 of HOST_TRANSFER_SLOTS=256), but metadata and tree operations (ListDir, Stat, WalkTree, StorageSnapshot, Rename, Remove, Mkdir, capabilities) all draw from a single global 32-permit blocking pool with no per-principal cap. blocking::spawn acquires the permit and the spawned closure holds it for the entire operation; a WalkTree or StorageSnapshot over a large exported tree holds its permit until the whole walk finishes. One authorized peer can open ~32 concurrent WalkTree/StorageSnapshot streams (QUIC allows 64 bidi streams per connection) and occupy every permit.

**Fehlerszenario.** A paired peer opens 32 simultaneous WalkTree requests on a large exported folder (or 32 slow ListDir requests against an exported saved SFTP connection via include_connections). All 32 control permits are held for the duration. Every other peer's ListDir/Stat/Rename/Delete, and this peer's own browsing, block on blocking::spawn until a walk completes - the Share host appears hung for metadata operations for all peers, while transfers (separate slots) keep running.

**Richtung.** Add a per-principal cap (and/or a fair queue) for control-pool operations, mirroring the per-principal transfer slots, so one identity cannot hold more than a small share of the 32 permits; consider a separate smaller pool or explicit quota for long-running WalkTree/StorageSnapshot so they cannot starve short ListDir/Stat calls.

**Gegenprüfung.** The code matches the claim. There is one process-wide Semaphore of 32 permits with no principal key (blocking.rs:16-23). spawn() moves the owned permit into spawn_blocking, so it is held for the whole closure (blocking.rs:46-53). ListDir, Stat, Rename, Promote, Mkdir, CreateDir, RemoveFile, RemoveDir and DiscardStage all go through control()/simple() into that pool (server_fs.rs:89-98,140-256,347-380). Capabilities and lease acquisition (server_capabilities.rs:36) and ReleaseLease (server.rs:274) use it too. WalkTree and StorageSnapshot run the entire traversal inside one permit (walk.rs:29-32,44-102; storage_snapshot.rs:26-32,89-130). That traversal is bounded only by MAX_WALK_NODES and a 60 s deadline per response (io_deadline.rs:7,30-35; walk.rs:119-121), so a client that keeps reading, or simply re-issues walks, can hold permits indefinitely. A peer may open 64 concurrent bidi streams per connection (keepalive.rs:8,112). Each stream is spawned with no per-connection or per-principal cap (server.rs:154-189; node_idle.rs:253-271 only counts streams). Transfers, by contrast, have per-principal slots (server_admission.rs:25-31,64-77). Even legitimate load contributes: an Android remote analysis keeps 8 ListDir RPCs in flight (analytics_backend.rs:152-190; backend.rs:434-436), so a few concurrent phone analyses already consume a large share of the 32 permits. I lower the severity to medium because the impact is availability-only. It is a temporary stall of Share metadata operations, with no data exposure, and it requires an authorized identity (an accepted Direct grant or any room-code holder).

## S38 Peer-triggered storage analysis adds a canonicalize()+symlink_metadata() syscall per directory that the local scan does not, slowing remote analysis (matches the phone->PC slowness report)

- Schwere: low · Urteil: partially_confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/os/shared/storage_analysis_host.rs:80-98; native/src/analytics/os/shared/analytics.rs:58-60,182-187,236; native/src/mobile/os/shared/domains/analyze.rs:189-211 (Android client bypasses host-side StorageAnalysis); native/src/analytics/os/shared/analytics_backend.rs:11-39,76,152-190; native/src/analytics/os/shared/remote.rs:6-20; native/src/app/core/analytics_core.rs:423-425; native/src/daemon/os/shared/ipc_analysis.rs:104-106; native/src/share/core/backend.rs:205-212,227-233,434-436

**Beschreibung.** When a peer runs storage analysis over a Direct/Room share, the host scans its own disk through scan_with_guard with a guard closure that, for every directory entered, calls local_access::symlink_metadata plus std::fs::canonicalize and a starts_with(root) check. The host's own GUI/CLI analysis of the same folder calls crate::analytics::scan(...) which passes guard=None and does none of this. Directory symlinks are already filtered during enumeration (is_link_like children are skipped), so the per-directory canonicalize is redundant defense-in-depth that doubles the per-directory syscall cost. On Windows canonicalize opens a handle and resolves the final path, so it is comparatively expensive; over a tree with hundreds of thousands of directories this is a large, directory-count-proportional slowdown that only the remote path pays.

**Fehlerszenario.** User connects a phone to a PC over Direct share and runs storage analysis on a big folder. The PC (host) runs the scan with the per-directory guard, performing one extra canonicalize + symlink_metadata per directory versus running the same analysis locally on the PC, so the remote-triggered analysis is noticeably slower for the same data - the behavior the user reported.

**Richtung.** Confine the expensive check to the root (and to the TOCTOU-sensitive case), or replace per-directory canonicalize with a cheaper identity check (e.g. verify the directory's own symlink_metadata is not link-like, which enumeration already guarantees, and validate the root once), so remote analysis has parity with local scan cost while keeping escape protection.

**Gegenprüfung.** The code facts are correct. The host-side guard (storage_analysis_host.rs:80-95) calls symlink_metadata, then std::fs::canonicalize and a starts_with check, and is passed as Some(&guard) at line 98. The local GUI scan uses scan(), which passes None (analytics.rs:58-60; analytics_core.rs:419-421). The guard runs on entry to every directory (analytics.rs:182-187). On Windows this costs extra handle opens plus a GetFinalPathNameByHandle per directory (read.rs:71-79; directory.rs:363-368). The causal link to the user's phone->PC slowness is refuted. Android analysis goes AnalysisViewModel.kt:125 -> AnalyzeApi.kt:162 -> domains/mod.rs:181 -> analyze.rs:166-211. For any location containing '://' (locations.rs:8-10) it calls crate::analytics::scan_backend directly (analyze.rs:207-209). scan_backend walks only by per-directory backend.list_dir (analytics_backend.rs:11-39,76), breadth-first, level by level, with 8 workers (152-190; PeerBackend parallelism 8, backend.rs:434-436). Each step is one ListDir round trip (backend.rs:205-212), re-resolved on the host with two canonicalize calls (fs.rs:146-183,311-344) through the 32-slot pool. The Android client never reaches PeerBackend::scan_storage (backend.rs:227-233). Only scan_remote calls it (remote.rs:6-20), and only the desktop GUI (analytics_core.rs:423-425) and the daemon IPC (ipc_analysis.rs:104-106) use scan_remote. So in the reported direction (phone as client, PC as host) the PC never executes scan_local_target or the guard, and the slowness comes from the per-directory network walk. Also, the guard is not purely redundant: canonicalize+starts_with detects an ancestor directory swapped for a junction or symlink after enumeration. The is_link_like filter applied at enumeration time (analytics.rs:236) cannot detect that, although the guard does not close the race either. The overhead is real only for desktop clients analysing a peer, hence low.

## S39 Dead/misleading 'block symlinks outside the share' checkbox: the flag is never read and confinement is unconditional

- Schwere: low · Urteil: confirmed · Kategorie: platform · Plattformen: Windows, Linux
- Stellen: native/src/app/core/share_exports_ui.rs:68-71 (checkbox bound to share_block_symlink_escape); native/src/app/core/state.rs:456 (field declaration); native/src/app/core/init.rs:445 (initialized true); native/src/share/core/fs.rs:324-344 (ensure_under_root always blocks escape)

**Beschreibung.** The desktop export UI shows a security checkbox 'Symlinks ausserhalb der Freigabe blockieren' bound to share_block_symlink_escape. That field is only ever written (declaration, init to true, the checkbox) and is never read anywhere in the codebase; the actual symlink/reparse confinement in secure_local_target/ensure_under_root is always enforced regardless of the toggle. Security is therefore not weakened, but the control is non-functional and misleading: a user could uncheck it believing they are permitting symlink following (nothing changes) or rely on it believing they enabled protection. It is also a latent trap if someone later wires it to disable confinement.

**Fehlerszenario.** A user unchecks the box to allow a share that contains symlinks to resolve them; nothing changes and symlinked content is still refused (confinement is hard-coded), with no feedback explaining why - a confusing, security-relevant control that does nothing.

**Richtung.** Either remove the checkbox, or wire it to a real, safe behavior and persist it in the export policy; if kept, it must never be allowed to disable the canonical-ancestor confinement for untrusted peers.

**Gegenprüfung.** A repo-wide rg (native/src and android/app/src) finds share_block_symlink_escape only at its declaration (state.rs:456), its initialization to true (init.rs:445) and the &mut checkbox binding (share_exports_ui.rs:68-71). There is no read site, and the value is not persisted, so it resets to true on every start. Confinement is unconditional. secure_local_target canonicalizes the root and ensure_under_root rejects any target that resolves outside it (fs.rs:311-344). The mount-lease path repeats the check (mount_lease.rs:130-134). Security is therefore not weakened, but the checkbox does nothing and is misleading. One minor correction to the failure scenario: unchecking the box does not change behaviour, and symlinks whose target stays inside the root are still allowed (fs.rs:325-330). Only escaping targets are refused, and walks and analysis skip links regardless (walk.rs:197-202,228-229; analytics.rs:236).

## S40 Single reads are not re-authorized per chunk; an un-shared file can keep streaming during the asynchronous connection teardown window

- Schwere: low · Urteil: partially_confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/server.rs:294-295 (admit_each = mutations or batches only); native/src/share/core/server_fs.rs:311-323 (read path: admit slot, no per-op re-auth); native/src/share/core/mount_lease.rs:389-406 (run() documents the auth model); native/src/share/core/node.rs:299-331 (invalidate_sessions closes connections asynchronously)

**Beschreibung.** Export/grant changes are enforced for in-flight work by closing all QUIC connections (invalidate_sessions) and, for mutations and batch entries, by re-checking authorization per operation (admit_each / MountLeaseAuthorization / BatchAuthority). Single Read/ReadAt requests on the stateless (Dynamic) path are authorized once at stream start and then stream to completion without any per-chunk re-check; revocation relies solely on the connection being torn down. Because the teardown is asynchronous relative to an already-running blocking read worker, a file that was just un-shared (or whose grant was revoked) can continue to be read for the brief window until the connection actually closes.

**Fehlerszenario.** A peer starts reading a large file; the owner immediately removes that export root or un-accepts the contact. invalidate_sessions bumps the epoch and queues connection close, but the read worker already streaming chunks continues until the stream send fails on close, so a bounded amount of just-revoked content is still delivered.

**Richtung.** For long reads, check an authorization epoch/cancellation token between chunks (as transfers already carry a stall deadline) so a revocation observed mid-stream aborts the read promptly instead of only on connection close; the window is small but closing it makes revocation deterministic.

**Gegenprüfung.** It is true that single reads are authorized once, at stream start (server.rs:221 session.authorize; lease check at 296-323 with admit_each false for reads, 294-295). The read then streams with no further authorization check (server_fs.rs:311-323; server_transfer.rs:72-134). Mutations and batch entries, by contrast, re-check through MountLeaseAuthorization::run and BatchAuthority (mount_lease.rs:389-406; server_fs.rs:59-69). The claimed 'asynchronous teardown window' is overstated, though. Every revocation path commits the new state and then, on the same thread, synchronously calls invalidate_sessions: configuration_runtime.rs:63-108 (state at 99, invalidation at 104), signal_commands.rs:445-472 for direct_online, and node.rs:342-352 for stopping Share. invalidate_sessions bumps the epoch, clears every mount lease and calls Connection::close on every tracked incoming connection (node.rs:300-329). Filesystem connections are registered before the handshake (server.rs:86). Under the QUIC library contract, close() is immediate: pending stream operations fail with LocallyClosed and unsent stream data is not transmitted. The read loop's next send_tagged therefore fails (server_transfer.rs:92-97), the receiver is dropped and the worker exits at its next blocking_send (130-132). The real exposure is only data handed to QUIC in the microseconds between the state commit and close(). The 512 KiB channel buffer is discarded, not delivered. One genuine but minor residual: saved connections exported through include_connections are resolved live from creds and are not part of ShareExportConfig (fs.rs:268-277). configuration_changed (authorization_policy.rs:6-17) therefore does not fire when a saved connection is removed, and in-flight operations on it continue.

## S41 Established peer connections are uncapped; the handshake limiter only bounds in-flight handshakes, letting one peer accumulate connections and multiply control-pool/transfer pressure

- Schwere: low · Urteil: partially_confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/node.rs:34-35 (MAX_PENDING_APPLICATION_HANDSHAKES=64, MAX_PENDING_HANDSHAKES_PER_ENDPOINT=4); native/src/share/core/server.rs:141 (handshake permit dropped once the session is accepted); native/src/share/core/node_accept.rs:20-52 (permit acquired only around the handshake); native/src/share/core/keepalive.rs:101-120 (each connection carries receive-window credit and up to 64 streams)

**Beschreibung.** Admission is bounded only during the handshake: a global 64-permit semaphore and a per-endpoint limit of 4 concurrent handshakes, and the permit is released as soon as the session is authenticated (server.rs:141). There is no cap on the number of fully established connections a single authorized peer may hold open (kept alive by the 5s keepalive). Each connection can open up to 64 bidi streams and carries its own QUIC receive-window credit. This amplifies the control-pool starvation (finding above) and lets one identity hold many connections' worth of per-connection state.

**Fehlerszenario.** A paired peer repeatedly opens connections (4 handshakes at a time; each permit frees on completion, so it can establish many over a short period) and keeps them alive, then spreads WalkTree/ListDir streams across them to exceed what a single connection's streams would allow, worsening metadata-pool starvation and raising host memory/credit use.

**Richtung.** Cap the number of concurrent established connections per principal (and globally), evicting or refusing beyond the limit, so post-handshake connection accumulation cannot be used to bypass per-connection stream limits or amplify resource use.

**Gegenprüfung.** It is true that admission limits apply only during the handshake (node.rs:34-35; node_accept.rs:21-43; handshake_limits.rs:43-70). The permit is dropped right after PeerHelloOk (server.rs:135-141), and incoming_sessions is insert-only with no per-principal count (node.rs:273-298). Idle connections are closed only by low-power sweeps (node_idle.rs:1-10,66-88), so an authorized peer can keep any number of established connections alive. The claimed amplification is mostly wrong. A single connection already allows 64 concurrent streams (keepalive.rs:8,112) against only 32 control permits (blocking.rs:16), so extra connections do not worsen control-pool starvation. Transfer slots are counted per principal across all of its connections (server_admission.rs:27-31,64-77,101-109). Mount leases are capped per principal (mount_lease.rs:10-11,250-259), and storage analysis is capped globally (storage_analysis_server.rs:23-26). What remains is per-connection QUIC state plus receive-window credit of 16-64 MiB per connection (keepalive.rs:21-24,89-99). That credit can be filled on streams the host has not read, so it multiplies with the number of connections: a memory-pressure vector, but only for an already-authorized identity.

## S42 Peer storage analysis is globally limited to 2 concurrent scans with no per-principal share, so one peer can block analysis for all others

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/storage_analysis_server.rs:23-26 (global Semaphore::new(2)); native/src/share/core/storage_analysis_server.rs:38-60 (acquire before scanning; each scan spawns a thread + rayon pool)

**Beschreibung.** StorageAnalysis requests share a single process-wide semaphore of 2 permits with no per-principal accounting. One authorized peer can hold both permits with two long analyses, delaying every other peer's analysis request (they wait, emitting heartbeats). Each admitted analysis also spawns a dedicated large-stack thread and its own rayon pool, so two peers can pin two full scan pipelines on the host.

**Fehlerszenario.** A paired peer starts two large storage analyses; a second peer's analysis request waits indefinitely (only heartbeating) until one of the first peer's scans finishes, even though the host has spare capacity it could fairly allocate.

**Richtung.** Make the analysis concurrency limit per-principal (or reserve at least one slot for other principals), and bound total analysis work per peer, so a single identity cannot occupy all analysis capacity.

**Gegenprüfung.** The code matches the claim. There is one global Semaphore::new(2) with no principal accounting (storage_analysis_server.rs:23-26). serve() acquires it before any work and meanwhile only heartbeats (36-44). The permit then moves into a dedicated thread with a 64 MiB stack for the entire scan (51-60; analytics.rs:45), and a local target also builds its own rayon pool (analytics.rs:90-98). The client keeps waiting as long as each frame, heartbeats included, arrives within 60 s (peer_storage_analysis.rs:64-77). A queued request therefore waits without bound while one peer holds both permits. Two caveats: the wait ends as soon as a holder's scan finishes or is cancelled (CancelOnDrop, storage_analysis_server.rs:18-21,30,65), so 'indefinitely' means only for as long as the holder keeps scanning. Also, only desktop and daemon clients that use scan_remote reach this path; Android clients never send StorageAnalysis (analyze.rs:207-209).

## S43 Signaling and relay run in plaintext by default; a configured TLS endpoint silently falls back to plaintext

- Schwere: high · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/signal_handshake.rs:36-55; native/src/share/core/signal_connected.rs:61-64; vendor/iroh-relay-1.0.0/src/server.rs:133-167; vendor/iroh-relay-1.0.0/Cargo.toml:71-81; share-server/src/transport.rs:79-98

**Beschreibung.** The server has only plaintext listeners: raw newline-JSON TCP and a ws:// upgrade on the same port, plus an Iroh relay served over plain http (the server crate has no TLS dependency). TLS exists only when the operator adds a reverse proxy and the user types wss:// or https://. The client treats bare host, host:port, tcp://, ws:// and http:// as plaintext. The desktop hint suggests host:port first, the CLI and Android accept plaintext schemes, and nothing warns the user. SignalConnection::connect tries the configured endpoints in order and moves to the next one on any error, including a TLS certificate failure. The documented list 'wss://share.example.com/se-share, share.example.com:51820' therefore drops to plaintext exactly when an active attacker blocks or intercepts the TLS endpoint. The relay URL derived from a TCP endpoint is http://host:port+1. In plaintext, an on-path party reads: the Hello (device id, device name, all local IPv4 addresses from lan_ips(), which the server ignores, public key), every presence (lookup and room IDs, node ID, LAN and public candidates, relay URL), and tracked envelopes including free-text messages and decisions. The same party can inject every unauthenticated server message: direct_offline, room_left, direct_route_ack, keepalive, and a hello_ok without tracked_direct_v1. This is a prerequisite or multiplier for the other findings. File data stays end-to-end encrypted by Iroh.

**Fehlerszenario.** The user enters 'share.example.com:51820' as the hint suggests, or the documented wss+TCP fallback list. On a hostile Wi-Fi the attacker resets TCP/443 or presents a bad certificate. The client silently connects to :51820 in plaintext. The attacker records room_id, lookup_ids, device IDs and IP candidates and can later abuse them remotely (hijack, eviction, forged rejections). The attacker can also strip tracked_direct_v1 from hello_ok and inject legacy decisions. The user sees 'Share-Server verbunden' with no security indication.

**Richtung.** Make TLS the default. Add a TLS listener option to se-share-server for signaling and the relay (static cert or ACME). Interpret scheme-less endpoints as wss:// and https://. Require an explicit, visibly warned opt-in for plaintext (for example 'insecure+tcp://') on desktop, CLI and Android. Never fall back from a TLS endpoint to a weaker one after a TLS or certificate error (only across endpoints of equal security). Offer certificate-fingerprint pinning for self-hosted servers. Drop the unused 'lan' field from Hello. Document that device names, request messages and discovery aliases are visible to the server.

**Gegenprüfung.** Server: main.rs:98 binds a plain std TcpListener and transport.rs:79-98 serves raw newline JSON or a ws upgrade on the same socket (tungstenite has only the 'handshake' feature, Cargo.toml:14). relay.rs:144-168 builds RelayConfig::new(), whose tls is None (vendor server.rs:133-167), so the relay serves plain HTTP/WS and logs http:// (relay.rs:121-124). Client: signal_connection.rs:71-83 sends bare host, host:port and tcp:// to raw TCP, and http:// becomes ws:// (341-350). connect() (53-69) returns the first endpoint that succeeds and reports the collected errors only if every endpoint fails. A wss certificate or handshake error from client_tls (130) therefore falls through silently to the next entry, and SHARE_SERVER.md:28-32 recommends exactly 'wss://..., host:51820'. session.rs:355-368 derives http://host:port+1 relay URLs from TCP entries. The Hello carries device id/name, lan_ips() (system.rs:1-24), the public key and the fingerprint (signal_connector.rs:35-51); the server discards lan (transport.rs:120-129). These server messages can be injected without authentication: direct_offline (signal_auth.rs:38-40), room_left (90-92), route ACKs that are only matched to a local outbox (tracked_signal_dispatch.rs:273-299) and keepalive (signal_connected.rs:347). A hello_ok without tracked_direct_v1 is accepted (signal_handshake.rs:36-55) and switches the client to the legacy request path (signal_publish.rs:48-54). Nothing warns the user: there is only the hint at menus_settings.rs:16 and a status line naming the transport label (signal_connected.rs:61-64). Corrections: (1) The app has no default server (an empty config means LAN-only, signal_worker.rs:81-87). 'By default' therefore means the server's only native mode plus the suggested host:port. The downgrade needs a plaintext entry in the endpoint list. (2) 'No TLS dependency' is inaccurate. iroh-relay's 'server' feature already pulls in rustls/ACME (vendor Cargo.toml:71-81) and supports RelayConfig.tls; share-server never sets it. (3) The Android placeholder is wss:// (SettingsScreen.kt:179), but validate_server accepts tcp/ws/http (share_settings.rs:43-46). File data stays end-to-end encrypted. High stands, because this is the precondition that lets any on-path party carry out #1-#3.

## S44 Anyone who knows a Direct lookup ID can take over its server routing, swallow requests and force 'offline'

- Schwere: high · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: share-server/src/protocol.rs:208-211; native/src/share/core/tracked_signal_sender.rs:61-76; share-server/src/idle_outbox.rs:297-305; native/src/share/core/tracked_signal_dispatch.rs:280-298

**Beschreibung.** publish() keys the server route by presence.relation_id and overwrites state.direct for any registered client. The only check is presence.device_id == the client's Hello device_id, and that value is itself unauthenticated. There is no ownership check, no proof of the Direct secret, and no conflict rejection. The new owner can unpublish, which deletes the route and sends direct_offline to every watcher. watch() hands the current presence to any client. The lookup ID is part of the Direct code, so every current and removed contact has it, as do the operator and plaintext observers. Routing also adapts to the hijacker's capabilities: a hijacker that did not negotiate tracked_direct_v1 receives requests as legacy direct_access_request. The requester then gets direct_route_ack 'legacy_forwarded', which permanently stops its automatic retries (only a manual retry remains). The idle outbox records route and expiry of whatever presence it forwarded, valid or not, when it decides whether to defer refreshes for sleeping phones.

**Fehlerszenario.** Mallory has the victim's Direct code (an ex-contact, or someone who once saw the code) or observed it in plaintext signaling. She connects without tracked_direct_v1 and sends publish_direct with relation_id = victim lookup and her own device_id. (a) Every new tracked request to the victim is delivered to her as a legacy request: she reads the requester's identity and message, and the requester's ledger marks it legacy_forwarded and never retries, so the victim never receives it. (b) Right after each genuine 60 s refresh she sends unpublish_direct. Watchers receive direct_offline, the daemon sets the contact Offline and presence=None, and there is no server route to dial the victim. (c) watch_direct gives her the victim's IP candidates and node ID indefinitely. (d) Idle Android watchers: a bogus refresh that copies the genuine route is recorded as 'held' with a fresh expiry. Her later offers replace the deferred genuine refresh before each tick, so the phone's copy expires (derived from code, not executed). A malicious server or a plaintext MITM can send the legacy_forwarded ACK directly.

**Richtung.** Require proof of the relation for publish, unpublish and watch without revealing the secret to the server. Option 1: derive a per-lookup Ed25519 key from the Direct secret with HKDF, make the server-visible lookup ID a hash of its public key, and require a signature over (lookup_id, device_id, server nonce or timestamp). Option 2: bind the lookup to the first publisher's identity key (TOFU) and verify signatures with it. Reject a publish while another live owner holds the lookup instead of replacing it. Encrypt presence route fields (node ID, candidates, relay URL) under the relation secret so the server and lookup-only parties learn nothing. On the client, treat direct_offline as a hint: keep the last valid presence until it expires. Do not stop automatic retries on legacy_forwarded unless the target is known to be legacy. Defer idle refreshes only on behalf of an authenticated owner.

**Gegenprüfung.** tracked_direct.rs:212-231 overwrites state.direct[relation_id] for any client whose Hello device_id equals presence.device_id. That id is unauthenticated: transport.rs:120-128 drops public_key/fingerprint, state.rs:60-102 has no ownership or uniqueness check, and the server cannot check the HMAC. unpublish removes the route only for the current owner, then sends direct_offline to every watcher (243-258), so the attacker publishes first and then unpublishes. watch() returns the stored presence to anyone (260-305). route_request routes by the owner's capability (43-85). The client attaches legacy_presence whenever it can (tracked_signal_sender.rs:61-76), so a non-capable owner receives DirectAccessRequest and the requester gets LegacyForwarded. direct_ledger.rs:207-211 then stops automatic retries; only a manual retry remains (247-258). The client applies direct_offline without verification (signal_auth.rs:38-40) and clears presence (ipc_host_events.rs:121-131). relay_event accepts a LegacyForwarded ACK for any local outgoing request (tracked_signal_dispatch.rs:280-298), so a server or MITM can forge it. (d) traced statically, not executed: may_defer compares only a route digest (idle_outbox.rs:248-279, 336-347), defer() replaces the waiting refresh (281-295), and flush() records whatever was delivered as held (297-305), even though the phone rejects the forged copy (signal_auth.rs:31). Corrections: in the legacy variant the hijacker gets only the requester's presence (Out::DirectAccessRequest = lookup_id + presence, protocol.rs:208-211), not the free-text message. She sees the full signed request including the message only if she negotiates tracked_direct_v1; the ACK is then Forwarded, the requester keeps retrying, and a retry reaches the victim once the genuine 60 s refresh takes the route back. She must therefore re-publish after each genuine refresh, which she can observe through her own watch. Lookup IDs are embedded in SE-D3 codes (profiles.rs:193-222) and change only on explicit regeneration (identity_store.rs:49). High stands: remote, repeatable, and open to ex-contacts and observers.

## S45 Room routing trusts anyone who knows a room_id: roster disclosure, member eviction and forged room_left

- Schwere: high · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: share-server/src/state.rs:145-146; share-server/src/writer.rs:220-270; share-server/src/limits.rs:18-26; share-server/src/limits.rs:200-233

**Beschreibung.** join_room validates field shapes and checks presence.device_id == the joiner's unauthenticated Hello device_id. The server cannot verify the HMAC. The joiner immediately receives a RoomRoster with every member's presence (device names, node IDs, public keys, LAN and public candidates, relay URLs) and all later room_joined broadcasts. If the device_id already exists in the room, the existing entry is silently reassigned to the new client and removed from the old client's room set; the evicted client is not told. When the new client leaves or disconnects, room_left for that device_id goes to all members. Room IDs come from the room code (SE-R3-<room_id>-<secret>), never rotate, and are visible to the operator and to plaintext observers. The 64-member cap can also be filled with fake members.

**Fehlerszenario.** An observer who once saw a join_room in plaintext (or an ex-member) joins with an arbitrary presence and gets the full roster: IP addresses and device names of all members, continuously. They then register with a victim member's device_id (taken from the roster) and join. The victim's server-side membership and roster updates move to them. They then leave_room. The other members receive room_left(victim) and their daemons persist the member as Offline with relay_url, candidates and presence cleared, so they cannot dial the victim until its next refresh. Repeating this after each refresh keeps the victim effectively offline. 63 fake members (8 sources × 8 registrations) make legitimate joins fail with 'too many room members'. Re-joins with changing presences fan out room_joined to every member and wake Android members.

**Richtung.** Derive a room signing key from the room secret: the server stores only its public key, or room_id = H(pubkey). Require signed join and leave messages over (room_id, device_id, server nonce or timestamp). Bind Hello device IDs to identity keys (see the device_id finding) and never reassign a member entry to a different key. Withhold the roster until the joiner has proven room possession. Optionally encrypt route fields under the room secret. On the client, ignore room_left while the member's last verified presence is still current, or require a signed leave.

**Gegenprüfung.** join_room (state.rs:104-205) checks field shapes and only client.device_id == presence.device_id (123). It then builds and sends every other member's full presence (135-144, 196) without any proof of the room secret. If the device id already exists, the old client's rooms entry is removed and the member slot is reassigned (157-166). The evicted client gets no message, and its room traffic now goes to the impostor's client id. leave_room and cleanup broadcast RoomLeft for whichever device id belonged to the leaving client id (213-243, 289-322). The client forwards RoomLeft without authentication (signal_auth.rs:90-92). The daemon clears status, relay, candidates and presence (ipc_host_events.rs:248-266), and merge_members persists that (ipc_host_profile_merge.rs:78-117). The victim's next refresh join (signal_publish.rs:56-67) restores the entry, so the attack has to be repeated after each refresh, as the finding describes. Room IDs are random 12-byte hex (profiles.rs:151-157). Attackers are therefore people who saw a code or a join: removed or locally blocked members (blocking is per device, session.rs:229), the operator, and plaintext observers. The member cap is 64 (limits.rs:16, state.rs:130-133) with 8 registrations per source (limits.rs:10). Correction, a cheaper DoS than the one described: join_room also rejects any join whose roster serializes above MAX_JSON_LINE = 256 KiB (state.rs:145-146, writer.rs:220-270). Field caps allow about 13.9 KB per presence (limits.rs:18-26, 200-233). The characters '"' and '\\' are not control characters and double in size under JSON escaping. About 10-19 oversized fake members (2-3 sources) therefore already make every new join and every legitimate refresh fail with 'too many room roster bytes', and peers' copies expire after 300 s. A presence with a changed route is sent to idle members at once (idle_outbox.rs:259-265). High stands.

## S46 Legacy direct_access_accepted: the accept/reject bit and message are unauthenticated and persistently flip Direct contacts

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/direct_reciprocal.rs:263-271; native/src/daemon/os/shared/ipc_host_events.rs:338-366; native/src/share/core/tracked_signal_dispatch.rs:30-38

**Beschreibung.** The client verifies only the presence attached to a legacy decision, never the decision itself. verify_direct_access_accepted ignores 'accepted' (the parameter is named _accepted) and 'msg'. It checks only that the attached presence is a current, HMAC-valid presence of the contact's target for that lookup, and that its nonce was not yet used under the 'direct-accepted:' replay key. The target's ordinary published presence passes, and its direct_available delivery uses a different replay key ('direct:'). The server forwards a legacy accept from any client to every connection registered with requester_device_id, with no sender, ownership or capability check. The handler also runs on sessions that negotiated tracked_direct_v1. The daemon applies it to any contact with that lookup, whatever its state, and persists it. The server can additionally omit tracked_direct_v1 from hello_ok; the client then silently falls back to the legacy request path.

**Fehlerszenario.** Mallory knows target T's lookup ID (any holder of T's Direct code, the operator, or a plaintext observer) and Bob's device ID. She sends watch_direct for T, receives T's current presence P, and sends {"t":"direct_access_accepted","lookup_id":T,"requester_device_id":Bob,"accepted":false,"presence":P,"msg":"Zugriff entzogen - neuen Code anfordern"}. Bob's daemon sets the long-established contact to Ignored with Mallory's text as status and persists it. peer_endpoint_source then refuses to open T ('Direktgeraet ist nicht mehr freigegeben') and reciprocal repair returns PolicyDenied. The tracked exchange is already complete, so no new signed decision arrives, and the relation stays broken until the user re-requests. With accepted:true she can flip a rejected or revoked contact back to Accepted (misleading UI; T still enforces its grants).

**Richtung.** Accept a legacy decision only for a contact whose outgoing request went over the legacy path and is still pending; never let it override a verified tracked decision. Bind the decision cryptographically: MAC the tuple (lookup_id, requester_device_id, accepted, msg, nonce) with the relation secret, or require an identity signature. Fail closed and warn when a server that previously advertised tracked_direct_v1 stops advertising it. Treat msg as untrusted text and do not show it as the contact status.

**Gegenprüfung.** verify_direct_access_accepted ignores accepted (signal_auth.rs:146) and msg. It only requires requester_device_id == own id (173-175) plus a current, HMAC-valid presence of the contact's target under replay key 'direct-accepted:...' (176-223). direct_available uses the separate key 'direct:...' (257), so the target's ordinary presence passes; watch hands that presence to anyone (tracked_direct.rs:283-304). decision_legacy validates shapes only and forwards to every client with that device id, with no capability or owner check (tracked_direct.rs:174-204, 411-419). The tracked parser ignores this tag (tracked_signal_dispatch.rs:30-38), so handle_server_msg runs for it even when tracked_direct was negotiated (104-119). The daemon applies the message to the contact with that lookup whatever its state: Ignored plus Failed(msg) (ipc_host_events.rs:172-202). It persists this (338-366) through merge (ipc_host_profile_merge.rs:25-40). Effects: opening is denied (peer_endpoint_source.rs:88-89), outgoing repair returns PolicyDenied (direct_reciprocal_coordinator.rs:74-76), and incoming reciprocal repair is refused as ContactIgnored (direct_reciprocal.rs:263-271). Recovery needs a new signed decision (direct_ledger_projection.rs:56-68) or a manual re-request (direct_actions.rs:269-284). A hello_ok without the capability is accepted silently (signal_handshake.rs:36-55). Severity lowered to medium: no access is gained (accepted:true only mislabels; the target still enforces its grants). The attack needs both the target's lookup ID and the requester's device ID. The harm is a persistent denial the user can repair, plus attacker-chosen status text that can be used for phishing.

## S47 Fixed, unauthenticated global ceilings let about 16 source addresses deny signaling to all users

- Schwere: high · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: share-server/src/writer.rs:220-270; share-server/src/rate_limits.rs:9-12; share-server/src/main.rs:125-132

**Beschreibung.** Registration needs only a syntactically valid Hello. Limits are 256 connection workers (16 per source), 128 registered clients (8 per source) and 128 accepts/s globally. With 16 IPv4 addresses, or 16 IPv6 /64s (one residential /56 has 256), an attacker holds all 128 registration slots indefinitely: raw TCP needs only a line every 60 s and WebSocket needs nothing. The remaining worker slots fill with silent pre-registration sockets that are reopened every 10 s. Eight sources at 16 accepts/s exhaust the 128/s global accept budget, so legitimate connects are shut down at accept. Every unregistered WebSocket socket busy-polls every 5 ms (about 200 wakeups/s each) for up to 10 s. join_room builds and serializes a roster of up to about 1 MiB (64 × 16 KiB presences) while holding the single global state mutex (outbound_fits). One connection re-joining at 128/s a 63-member room the attacker filled himself therefore stalls every other client. The documentation claims /64 grouping prevents rotation within a normal IPv6 allocation, but end-site allocations are typically /48 to /56. Side effect: legitimate users behind CGNAT or a shared office IP hit the 8-registration and 16-connection per-source caps.

**Fehlerszenario.** An attacker with 16 cheap VPS IPs opens 8 registered TCP connections per IP with random device IDs and sends a heartbeat every 30 s. Every legitimate Hello is answered with 'server client limit reached'. Presence, Direct requests, rooms and discovery are unavailable for everyone until the attacker stops. No secret or identifier of any user is needed.

**Richtung.** Authenticate Hello with the identity key (see the device_id finding) and cap registrations per proven key, not only per IP. Make the limits configurable and size them to the host. Under pressure, evict unauthenticated or subscription-less registrations first. Replace 5 ms polling with blocking reads with deadlines, or an async runtime. Clone presences under the lock but serialize after releasing it, and rate-limit join and leave per client. Correct the IPv6 statement in the docs and aggregate IPv6 sources more coarsely (/56) or adaptively.

**Gegenprüfung.** Limits are at limits.rs:7-10. Registration needs only a parsable Hello with protocol 3 and a non-empty device id (transport.rs:120-155, state.rs:66-83). Full/SourceFull then rejects legitimate Hellos. A non-idle raw TCP session needs one line per 60 s (idle.rs:17, transport_serve.rs:38). A non-idle WebSocket session has no inbound deadline (transport_serve.rs:73, signal_session.rs:57-59). The accept loop takes a worker permit, then the accept bucket (main.rs:125-132): 128/s global, 16/s per source (rate_limits.rs:9-12). Eight sources at 16/s therefore use up the global budget. Before registration, WebSocket sockets are non-blocking and sleep 5 ms per poll, both during the upgrade (transport.rs:265-291) and while waiting for the Hello (358-366), for up to 10 s. IPv6 is grouped by /64 (limits.rs:44-52); the documentation's claim about this (SHARE_SERVER.md:76-78) is as the finding states. Corrections: the roster work under the global mutex is a clone of up to about 63 x 13.9 KB (~0.9 MB) plus serialization that aborts at 256 KiB (writer.rs:220-270), not a full 1 MiB serialization. One connection re-joining at 128/s adds latency, but several attacker connections (8 per source are cheap) are needed to saturate the mutex. 'One connection stalls every other client' is overstated. The main attack, 16 IPv4 addresses or 16 /64s holding all 128 registrations, needs no secret. High stands.

## S48 The Iroh relay is open to any endpoint, and behind the documented TLS proxy its per-source cap limits the whole relay

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: share-server/src/relay.rs:246-264; vendor/iroh-relay-1.0.0/src/server/connection_source.rs:11-22

**Beschreibung.** RelayAccess::on_connect admits every authenticated EndpointId, limited only by 512 connections globally and 4 per endpoint. There is no link to signaling registration and no token. Endpoint keys cost nothing to generate, so the per-endpoint cap does not slow an attacker. 8 sources × 64 TCP connections fill all 512 slots, and 8 sources × 32/s exhaust the 256/s accept budget. Anyone on the Internet can push their own Iroh traffic through the operator's server at up to 64 MiB/s per connection. Behind the documented Caddy/nginx setup, every relay socket comes from 127.0.0.1, so the per-source cap of 64 TCP connections (and 32 accepts/s) applies to the entire relay. Every running Smart Explorer endpoint holds a home-relay connection (assumption: normal iroh behavior, which the app relies on via home_relay_connected). More than 64 online devices therefore exhaust the relay, and the nginx example's 8 connections per IP lets 8 attacker IPs do it. Signaling has SE_SHARE_TRUSTED_PROXY_IPS; the relay has no equivalent.

**Fehlerszenario.** The attacker runs 8 VPS, each opening 64 relay connections with fresh keys that answer pings. New legitimate relay connections get 'relay connection capacity reached'. Phones behind CGNAT or symmetric NAT that need relay fallback cannot reach their peers, and relayed transfers fail. In a TLS deployment behind nginx, the 65th concurrently online device cannot get a home relay even without an attacker.

**Richtung.** Admit relay connections only for endpoint IDs currently registered on the signaling server with a proven key, or for short-lived tokens issued over signaling (ClientRequest and AccessControl support this). Add a trusted-proxy mode for the relay (PROXY protocol, or X-Forwarded-For only from configured proxy IPs) so per-source limits apply to the original clients. Add global and per-key bandwidth budgets. Make the limits configurable.

**Gegenprüfung.** RelayAccess::on_connect admits any authenticated EndpointId, limited only by 512 connections in total and 4 per endpoint (relay.rs:201-219, AdmissionCounts 246-264). It has no link to signaling registration. TCP admission is keyed by the socket's peer IP (vendor http_server.rs:523-541, connection_source.rs:11-22): 512 in total, 64 per source, 256/s globally and 32/s per source (relay.rs:19-26, 152-161). Unlike signaling (limits.rs:61-92), there is no trusted-proxy classifier. Each connection may receive 64 MiB/s with an 8 MiB burst (relay.rs:28-29, 144-150). Behind the documented proxy all relay sockets share one source, which the documentation itself acknowledges (SHARE_SERVER.md:133-135). The nginx example allows 8 connections per client IP (lines 237-252). The one-home-relay-connection-per-device assumption is standard iroh behaviour and was not executed. Severity lowered to medium: this affects availability and enables bandwidth abuse only. Relayed traffic stays end-to-end encrypted and direct paths keep working. Open relays are iroh's default model (RelayConfig::new uses AllowAll, server.rs:161-167). The 64-device ceiling behind a proxy is documented.

## S49 Hello device_id is unauthenticated, so envelopes addressed by device are copied to impostors

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: share-server/src/transport.rs:120-149; share-server/src/transport.rs:199-237; share-server/src/state.rs:60-102; share-server/src/tracked_direct.rs:87-142; share-server/src/tracked_direct.rs:174-204; share-server/src/tracked_direct.rs:340-350; share-server/src/tracked_direct.rs:392-419; native/src/share/core/direct_ledger.rs:199-232

**Beschreibung.** register_client stores the Hello device_id without any proof of the identity key (the public_key and fingerprint fields are discarded) and allows any number of registrations with the same device_id. Request receipts, decisions and decision receipts (all signed) and legacy accepts are fanned out to every client whose Hello device_id matches. forward_to reports 'forwarded' as soon as any matching writer accepted the message.

**Fehlerszenario.** Mallory registers with Alice's device_id, which is visible to Alice's contacts, room members, the operator and plaintext observers. Every signed request receipt, decision and decision receipt addressed to Alice (both identities, node IDs, free-text messages, revisions) and every legacy accept, which carries a fresh target presence usable for the forged-decision attack, is also delivered to Mallory. While Alice is offline, Mallory's copy makes the server return 'forwarded'. Receipt envelopes stop retrying after a forwarded ACK, so Alice receives them only through later duplicate-request self-healing. The same impostor registration is what enables the room-eviction attack.

**Richtung.** Add a challenge-response to Hello: the client signs a server nonce, the server address and a timestamp with its Iroh identity key. The server binds device_id to that key (TOFU per device_id, or device_id = H(pubkey) for new identities), rejects concurrent registrations under the same device_id with a different key, and routes device-addressed envelopes only to clients that proved the key.

**Gegenprüfung.** register_client stores the Hello device_id without proof of key possession: transport.rs:120-128 discards public_key and fingerprint, and state.rs:60-102 has no uniqueness check, only per-source and global caps. Request receipts and decisions go to every capable client with requester.device_id, and decision receipts to every capable client with target.device_id (tracked_direct.rs:87-142, 392-400). Legacy accepts go to every client with requester_device_id, whatever its capabilities (192-203, 411-419). forward_to reports Forwarded as soon as any writer accepted the message (340-350). Receipt envelopes stop retrying on Forwarded (direct_ledger.rs:199-232). An impostor who is online while Alice is offline therefore suppresses receipt retries until Alice's duplicate request triggers the self-healing requeue. The signatures prevent forgery, so the impact is disclosure of envelope contents (identities, node ids, messages) plus delay, and support for the room eviction in #2. Medium stands.

## S50 Any registered client can block public discovery offers (pairing DoS)

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: share-server/src/discovery.rs:145-168; share-server/src/discovery.rs:170-245; share-server/src/discovery_state.rs:15-27; share-server/src/discovery_state.rs:75-155; share-server/src/discovery_state.rs:383-397; native/src/share/core/discovery_signal_state.rs:304-317; native/src/share/core/discovery_signal_exchange.rs:155-178; native/src/share/core/discovery_signal_dispatch.rs:191-235

**Beschreibung.** list() returns every offer to every capable client. Every StartPairing, including bogus ones, counts against the per-offer budget of 12 starts per 60 s before the publisher has validated anything. Each offer allows 4 concurrent exchanges, which live up to 2 minutes. The global offer and exchange tables hold 256 entries each, with 8 per client.

**Fehlerszenario.** One attacker connection lists the offers. For each target offer it sends 4 StartPairing messages with well-formed KE1 payloads (any PIN) and never sends KE3. The real connector gets 'discovery offer pairing limit reached' for 2 minutes. Re-arming costs 2 starts per minute, so the offer stays blocked for its whole lifetime; the publisher's own MAX_PUBLISHER_EXCHANGES_PER_OFFER of 4 is consumed too. Alternatively, 12 bogus starts at the beginning of each window give legitimate users 'attempt rate exceeded'. 32 registrations (4 sources) holding 8 offers each fill the 256-entry global table, and nobody can publish. Each start also wakes the publishing phone (see the wake finding).

**Richtung.** Charge start budgets per connector identity or source, not only per offer. Shorten the KE2-to-KE3 deadline to 15-30 s and free the slot as soon as the publisher rejects. Cap offers per source or identity. Offer unlisted offers that can only be found with a short offer code, so they cannot be enumerated.

**Gegenprüfung.** list() returns every offer except the caller's own (discovery.rs:145-168). prepare_exchange_locked caps active exchanges at 4 per offer and records every start in recent_starts before the publisher sees it (discovery_state.rs:108-141). An exchange lives until min(offer deadline, now + 2 min) (148). start_pairing checks only valid_text on the payload (discovery.rs:187-199), so 12 garbage starts per window are enough to trigger 'attempt rate exceeded'. The publisher accepts any well-formed KE1 and answers with KE2, counting against its own 4-per-offer and 12/min budgets (discovery_signal_exchange.rs:143-231, checks 155-178; discovery_signal_state.rs:304-317). Its deadline is now + 2 min (discovery_signal_dispatch.rs:243). Four KE1-only starts re-armed every 2 minutes therefore keep the offer blocked. The global limit is 256 offers with 8 per client (discovery_state.rs:15-16, 383-397), so 32 registrations from 4 sources fill it. Every handled discovery message requests a 15 s hold on the publisher (tracked_signal_dispatch.rs:70-76). Medium stands.

## S51 Online PIN guessing against discovery offers is rate-limited but never capped

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/discovery_signal_state.rs:22-23; native/src/share/core/discovery_signal_state.rs:304-317; share-server/src/discovery_state.rs:26-27; share-server/src/discovery_state.rs:131-141; native/src/share/core/discovery_exchange_port_impl.rs:172-176; docs/SHARE_SERVER.md:36-60

**Beschreibung.** Both the server and the publisher allow 12 pairing starts per offer per minute. A failed OPAQUE KE3 is only a protocol error: there is no failure counter, lockout or offer withdrawal. Offers may run for any positive duration (the client renews 5-minute server leases until the local deadline), and the PIN has no minimum length. The server operator bypasses the server-side limit and is bound only by the publisher's 12 per minute. OPAQUE itself prevents offline guessing, so this is purely an online-guessing budget.

**Fehlerszenario.** A user publishes a Direct offer for 24 h with a 4-digit PIN. Any registered client, or the operator, makes 12 guesses per minute, about 17,280 per day, which exceeds the 10,000 possible PINs. The attacker pairs successfully and obtains a persisted reciprocal Direct relation with access to the Standard-Direkt export, or, for a Room offer, the room material and thus full membership.

**Richtung.** Count failed KE3 verifications per offer. After a small number (for example 5-10), withdraw or lock the offer and show the user a warning. Tie the allowed offer duration to PIN length or entropy, or enforce a minimum for long offers. Add exponential backoff between attempts per offer.

**Gegenprüfung.** Server and publisher each allow 12 starts per offer per 60 s (discovery_state.rs:26-27, 131-141; discovery_signal_state.rs:22-23, 304-317). A wrong PIN only makes state.finish fail with protocol_error (discovery_exchange_port_impl.rs:172-176). There is no failure counter, lockout or offer withdrawal anywhere in native/src/share; a grep for such logic finds nothing. Offer duration is unbounded (SHARE_SERVER.md:36-45). PINs have no minimum length; the UI only warns about an empty PIN and '0' (lines 47-51). 12/min x 1440 = 17,280 online guesses per day, enough to exhaust a 4-digit PIN. OPAQUE prevents offline guessing. With the 5-minute default the budget is about 60 guesses, so the real risk comes from long offers combined with short PINs. Medium stands.

## S52 Unauthenticated server messages keep Android awake: power holds are requested before verification

- Schwere: medium · Urteil: confirmed · Kategorie: platform · Plattformen: Android
- Stellen: native/src/share/core/tracked_signal_dispatch.rs:62-77; native/src/share/core/tracked_signal_dispatch.rs:82-96; native/src/share/core/tracked_signal_dispatch.rs:104-113; native/src/share/core/signal_session.rs:23-43; native/src/share/core/signal_connected.rs:316-349; native/src/share/core/power.rs:25-30; native/src/share/core/power_hub.rs:90-115; native/src/share/core/power_hub.rs:145-158; share-server/src/tracked_direct.rs:144-172

**Beschreibung.** The client requests a 15 s hold (SIGNAL_ACTIVITY_HOLD_MS) in three cases, all before or regardless of verification: (1) any line whose 't' is direct_access_request or direct_access_accepted; (2) any tracked request or decision, even when verification then fails; (3) any discovery message the dispatcher handles. Each server keepalive causes a 5 s hold, an ack, an idle-connection sweep and a home-relay check, with no rate limit. The server forwards legacy access requests to the owner of any lookup without a capability check, and legacy accepts to any device_id.

**Fehlerszenario.** An attacker who knows the phone's lookup ID sends one malformed request_direct every 10 s through the server. Alternatively the attacker uses the phone's device ID with direct_access_accepted or a tracked decision, or starts pairings against the phone's public offer. A malicious server or plaintext MITM can do the same with a stream of keepalive messages. Every forwarded line asks the host hook for a partial wakelock, the background phone never sleeps, and the battery drains. Verification fails silently, so the user sees nothing.

**Richtung.** Request holds only after a message has been verified, and for discovery only for exchanges in an expected local state. Honor at most one server keepalive per negotiated interval. Count invalid messages and reconnect or back off when they repeat.

**Gegenprüfung.** tracked_signal_dispatch.rs:108-113 requests SIGNAL_ACTIVITY_HOLD_MS (15 s, power.rs:30) for any direct_access_request or direct_access_accepted line before handle_server_msg verifies it. Lines 82-95 request a hold after a tracked request or decision even when verification failed, and 70-76 do so for every handled discovery message. Each keepalive line causes a 5 s hold, an ack, a connection sweep and a relay check, with no rate limit (signal_session.rs:23-43, reached via signal_connected.rs:284-286 and 347). The server forwards legacy requests to any lookup owner without a capability check (tracked_direct.rs:144-172) and legacy accepts to any device id (174-204). Both reach idle clients immediately, because only presence refreshes are deferred (idle_outbox.rs:184-219). power_hub.rs:145-158 drops only requests within 5 s that extend the hold by at most 5 s. One forwarded line every 10-14 s therefore keeps re-arming the 15 s hold. The Android hook adds no rate limit (WakeKeeper.kt:44-63), and share_power.rs:28-33 also wakes the poller each time. Medium stands.

## S53 The server's 128-message burst limit is below the client's normal publish burst, causing an endless reconnect loop

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/signal_commands.rs:112-137; share-server/src/transport.rs:114

**Beschreibung.** The server drops a registered connection (PermissionDenied) as soon as one inbound message exceeds the 128-token bucket, which refills at 128/s. On every connect and every 60 s refresh, publish_all sends back to back: Hello, publish_direct, one watch_direct per auto-connect contact, legacy requests, and one join_room per auto-join room. send_pending_tracked then adds every due envelope. The server's own per-client limits allow 256 watches and 64 rooms, so legitimate configurations exceed the burst.

**Fehlerszenario.** A user has 130 auto-connect contacts, or 100 contacts and 30 rooms. After about 127 lines the server closes the connection, so the remaining subscriptions are never registered. The negotiation had succeeded, so the client resets its backoff to 1 s and reconnects every 1-2 s forever, with a 15 s CONNECT_HOLD power hold each time on Android.

**Richtung.** For registered clients, throttle reads instead of disconnecting, or size the burst to at least 1 + MAX_WATCHES + MAX_ROOMS + MAX_PUBLISHED + a margin. Add batched subscribe and join messages. Pace sends on the client with a token bucket that mirrors the server. Reset the reconnect backoff only after a session has stayed healthy for some time.

**Gegenprüfung.** The per-connection bucket holds 128 messages and refills at 128/s (rate_limits.rs:14-15). The Hello is charged to the same bucket (transport.rs:114, 336), and any line over budget ends the session (transport_serve.rs:57 for TCP; transport.rs:373-393 for WebSocket). publish_all sends PublishDirect, one WatchDirect per auto-connect contact and one JoinRoom per auto-join room, with no pacing (signal_publish.rs:16-69). It runs on connect, followed by send_pending_tracked (signal_connected.rs:84-104), and on every refresh (signal_session.rs:212-218; 60 s per keepalive.rs:27). The server's own limits allow 256 watches and 64 rooms (limits.rs:14-15), so more than about 126 subscriptions exceed the burst. After a successful negotiation the backoff resets to 1 s (signal_worker.rs:155-158, 169-185). The result is a reconnect loop every ~1-2 s, with a 15 s CONNECT_HOLD each time in low power (signal_worker.rs:137). ConfigureProfiles also runs publish_all (signal_commands.rs:112-137), so bursts recur more often than every 60 s on desktops (see unresolved). The precondition is uncommon; medium stands.

## S54 Silent WebSocket clients are never expired, so registration slots leak

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: share-server/src/transport.rs:304-316; share-server/src/transport.rs:347-353

**Beschreibung.** Non-idle WebSocket sessions have no inbound deadline (active_window None). The server never sends WebSocket pings to them, and accepted sockets have no TCP keepalive. A half-open connection is removed only if a write to it eventually fails, which never happens when no traffic is routed to it. Raw TCP sessions (60 s window) and idle sessions (server keepalive) are bounded.

**Fehlerszenario.** Direct ws:// deployment (a documented transport). A laptop on the home network sleeps or changes networks while connected 8 times, and its FIN never reaches the server. That leaves 8 zombie registrations for the home IP, so the next device or reconnect from home gets 'server source client limit reached'. Over time zombies also eat into the global limit of 128.

**Richtung.** Send WebSocket Ping frames (or an app-level keepalive) to non-idle clients every 30-60 s and close after a missed reply. Alternatively apply the raw-TCP 60 s inbound window to WebSocket too, since clients send a heartbeat every 20 s anyway. Enable TCP keepalive on accepted sockets.

**Gegenprüfung.** Non-idle WebSocket sessions run with active_window None (transport_serve.rs:73). poll() then returns Wait(None) (server signal_session.rs:57-59), and the loop reads with a 500 ms timeout forever. The server never sends WebSocket pings; it only answers client pings (transport.rs:347-353). share-server/src sets no TCP keepalive anywhere. The session ends only on EOF/error or a failed write (transport.rs:304-316, 331). Raw TCP is bounded by its 60 s window (transport_serve.rs:38), and idle sessions by keepalive plus a 60 s reply window (idle_outbox.rs:135-163, signal_session.rs:50-78). Qualification: zombies that still receive routed traffic (refreshes of watched contacts, room events) are cleaned up once the kernel's retransmission timeout fails a write (about 15 min on Linux). Only zombies that receive no routed traffic stay forever. Behind a trusted proxy they count only against the global 128 registrations, unless the proxy closes idle tunnels (the nginx example sets 300 s). Medium stands.

## S55 Presence device_name and fingerprint are not covered by the HMAC, and the MAC encoding is not injective

- Schwere: low · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/crypto.rs:135-153; native/src/daemon/os/shared/ipc_host_events.rs:445-467; native/src/share/core/session.rs:233-235; native/src/share/core/direct_reciprocal_coordinator.rs:100-106; native/src/share/core/peer_endpoint_source.rs:91-92

**Beschreibung.** presence_payload covers kind, relation ID, device ID, public key, node ID, relay URL, sorted candidates, expiry and nonce. It does not cover device_name or fingerprint. Fields are joined with '|' and candidates with ',' without escaping or length prefixes. Verified presences are persisted with these unauthenticated fields, and the stored fingerprint is later compared verbatim in session, repair and mount pin checks.

**Fehlerszenario.** A malicious server or plaintext MITM rewrites the device_name of room members; it is persisted as member.device_name and shown in the UI (for example when the user decides which device gets an Exec grant, an assumption since the UI was not inspected). Rewriting the fingerprint field makes room sessions fail with 'Raumgeraet hat ungueltigen Fingerprint', reciprocal repair report IdentityConflict, and mounts fail with 'Pins wurden geaendert', all misleading security errors. Merging candidates 'A','B' into one candidate 'A,B' keeps the MAC valid but breaks the direct routes.

**Richtung.** Introduce a versioned, length-prefixed (or canonical-JSON) MAC payload that includes device_name and fingerprint. Alternatively always derive the fingerprint locally from public_key and never store the transmitted value.

**Gegenprüfung.** presence_payload covers kind, relation id, device id, public key, node id, relay URL, sorted candidates joined by ',', expiry and nonce. The fields are joined by '|' without escaping or length prefixes (crypto.rs:135-153). device_name and fingerprint are set outside the MAC (signal_presence.rs:33-46). Direct verification checks the key against the code's expected fingerprint, not presence.fingerprint (signal_auth.rs:248); room verification checks neither (279-320). The daemon persists room members' device_name and fingerprint from the presence (ipc_host_events.rs:460-461, 470-472) and later compares the stored fingerprint (session.rs:234; direct_reciprocal_coordinator.rs:103-105; peer_endpoint_source.rs:91-92, 121-122). Candidates ['A','B'] and ['A,B'] produce identical MAC input. Only the operator or a plaintext MITM can exploit this, and they can already drop messages. The impact is UI spoofing of device names and misleading security errors. Low stands.

## S56 The client's WebSocket transport accepts 64 MiB messages from the server

- Schwere: low · Urteil: confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/signal_connection.rs:111-133; native/src/share/core/signal_connection.rs:254-322; native/src/share/core/line.rs:4

**Beschreibung.** connect_ws calls tungstenite's client_tls with the default WebSocketConfig: max_message_size 64 MiB and max_frame_size 16 MiB in tungstenite 0.24. The raw TCP path limits lines to 256 KiB. drain_messages buffers up to 256 messages before any of them is processed.

**Fehlerszenario.** A malicious server, or a MITM on ws://, streams 64 MiB text frames. The client buffers up to 256 of them per drain, allocating hundreds of MB or more. The Android process is killed by the low-memory killer, or the desktop daemon's memory spikes.

**Richtung.** Use client_tls_with_config with max_message_size and max_frame_size set to MAX_SIGNAL_LINE, and limit the total bytes buffered per drain.

**Gegenprüfung.** connect_ws calls client_tls(request, stream) (signal_connection.rs:130). That is client_tls_with_config(..., None, None) (tungstenite-0.24.0/src/tls.rs:163-172), so WebSocketConfig::default applies: max_message_size 64 MiB and max_frame_size 16 MiB (protocol/mod.rs:75-86). Raw TCP lines are capped at 256 KiB (line.rs:4; signal_connection.rs:258). drain_messages collects up to 256 messages before any is processed (signal_connection.rs:292-322); this path is used when a readiness watcher exists (signal_connected.rs:246-258). Memory use is bounded by what the peer actually sends. The raw TCP path can also buffer up to 256 x 256 KiB = 64 MiB per drain. Exploiting this needs a malicious server or a ws:// MITM. Low stands.

## S57 Server-to-client messages over WebSocket wait up to 500 ms

- Schwere: low · Urteil: confirmed · Kategorie: performance · Plattformen: Windows, Linux, Android
- Stellen: share-server/src/transport_serve.rs:25-27; share-server/src/transport_serve.rs:79-91; share-server/src/transport.rs:304-332

**Beschreibung.** For WebSocket clients, queued outbound messages are written only by the connection's reading thread, and only after its blocking read returns (read timeout up to 500 ms). Raw TCP connections have a dedicated writer thread and send immediately.

**Fehlerszenario.** Discovery pairing relays 6 packets, and the tracked request/receipt/decision/receipt flow relays 4. Through WebSocket peers each hop gains up to 0.5 s, about 3 s per pairing. Presence updates and request delivery to wss clients are delayed in the same way.

**Richtung.** Give WebSocket connections a separate writer (split the socket, or use an async runtime), or wake the reader when its outbound queue becomes non-empty.

**Gegenprüfung.** WebSocket writers have no writer thread (writer.rs:91-94), unlike the TCP writer thread (96-115). Queued messages are flushed only by the connection's own loop before each read (transport_serve.rs:79; transport.rs:331). With no deadline pending, that read waits up to WS_POLL_INTERVAL = 500 ms (transport_serve.rs:25-27, 80-83). Messages caused by the session's own inbound line, such as route ACKs, are flushed right after dispatch. Messages routed from other clients wait up to 500 ms (about 250 ms on average) per hop to a WebSocket peer. About 3 s is therefore the worst case for the 6 relayed pairing packets. Low stands.

## S58 Linux and Android store Share private keys, relation secrets and remote passwords unencrypted at rest (no DPAPI/keystore equivalent)

- Schwere: medium · Urteil: partially_confirmed · Kategorie: security · Plattformen: Linux, Android
- Stellen: native/src/creds/mod.rs:5-7; native/src/creds/os/linux_os.rs:1-5; native/src/creds/os/linux_os.rs:12-14; native/src/creds/os/linux_file_store.rs:324-335; native/src/creds/os/linux_file_store.rs:337-382; native/src/share/core/identity.rs:7-8; native/src/share/os/shared/profile_store.rs:110-138; native/src/share/os/shared/profile_store.rs:304-316; native/src/creds/os/shared.rs:244-252; native/src/support_dirs.rs:72-79; native/Cargo.toml:123-125

**Beschreibung.** On Linux and Android the credential backend is a plain file store (creds/os/linux_os.rs, selected by #[cfg(not(windows))]). encode_record/decode_record (linux_file_store.rs:324-382) write the secret verbatim after an 8-byte magic, a version byte, the account digest and a length, followed by a SHA-256 of the record for integrity only - there is no encryption. These records hold the Iroh identity private key (account 'share:identity:iroh_secret'), every Direct/Room relation secret (used for the session-proof HMAC that authorizes peer file access), and all saved remote-backend passwords. The module header (linux_os.rs:16-20) states this explicitly: 'Linux filesystem ownership and mode bits are the security boundary; encrypted home storage is required for offline secrecy.' Windows instead uses keyring windows-native, i.e. DPAPI (creds/os/windows.rs), which encrypts at rest tied to the user login.

**Fehlerszenario.** A Linux desktop without full-disk encryption is backed up, imaged, or the disk is removed. An attacker reads ~/.local/share/smart_explorer/secrets-v1/*.secret directly from the offline copy and recovers, in cleartext, the Iroh identity key, all Direct/Room secrets (which let them impersonate the device to peers and pass the session-proof HMAC) and every saved SFTP/FTP/WebDAV password. The same bytes on Windows would be DPAPI-protected and unreadable without the user's login.

**Richtung.** Treat this as a deliberate, documented limitation but close the gap for 'secure by default': derive a key from an OS keystore where available (kernel keyring / libsecret on Linux desktop, Android Keystore via the JNI host) and encrypt the file-store records with it, or at minimum surface the at-rest exposure to the user. Keep the 0600/0700 filesystem enforcement as defense-in-depth.

**Gegenprüfung.** LINUX: CONFIRMED. creds/mod.rs:5-7 selects os/linux_os.rs for every non-Windows target. encode_record (linux_file_store.rs:324-335) writes `secret.as_bytes()` verbatim after the header. The SHA-256 trailer only checks integrity, and decode_record (:337-382) hands back the plaintext. The design is deliberate and documented: linux_os.rs:3-5 (not :16-20; line 16 is `description()`), Cargo.toml:123-125 and docs/FILE_OPS_MATRIX.md:122-127 ('not encryption against ... offline access to an unencrypted disk').

The stored contents match the claim. identity.rs:7 'share:identity:iroh_secret' and :8 the direct-secret prefix. Relation secrets are written through profile_store.rs:110-138 (prepare_unique_credential) and read at :304-316. The cited profile_store.rs:290 is the profile-JSON save, not a secret write. Saved connection passwords go through creds/os/shared.rs:244-252. Windows uses Credential Manager through keyring windows-native (creds/os/windows.rs:22-62, Cargo.toml:125).

ANDROID: REFUTED. data_home is app.filesDir (InitConfig.kt:32 -> mobile/os/shared/init.rs:60-61 -> support_dirs.rs:57-61). That is app-private credential-encrypted storage, which the platform encrypts and sandboxes on minSdk 30 devices (build.gradle.kts:86). Cloud backup and device transfer are disabled (AndroidManifest.xml:51-52, data_extraction_rules.xml). An offline disk image does not yield these bytes.

SEVERITY: The stated offline-image/backup scenario alone is a documented, conventional Linux trade-off (low). Medium is still justified for a reason the finding misses. On Linux the plaintext store lives at $HOME/.local/share/smart_explorer/secrets-v1 (support_dirs.rs:72-79, linux_os.rs:13). That path is inside the default Share export root 'Home' (see #6). Share confinement only checks the path stays under the root (fs.rs:311-344), and dot-directories are allowed (fs_paths.rs:14). So any accepted Direct peer or room-code holder can download *.secret over Share and recover the Iroh key, every relation secret and every saved password. A login-secret-encrypted OS keyring would make that remote read useless, as DPAPI does on Windows.

## S59 Windows daemon IPC token/address/generation files are not owner- or ACL-verified (Linux strictly enforces uid+mode); token is the only gate to the full Share control plane

- Schwere: low · Urteil: partially_confirmed · Kategorie: security · Plattformen: Windows
- Stellen: native/src/daemon/os/windows/ipc_storage.rs:109-127; native/src/daemon/os/windows/ipc_storage.rs:129-147; native/src/daemon/os/windows/ipc_storage.rs:200-223; native/src/support_dirs.rs:64-70; native/src/daemon/os/linux_os/ipc_storage.rs:130-190; native/src/daemon/os/linux_os/ipc_storage.rs:262-283

**Beschreibung.** The daemon control plane is authenticated solely by the shared token in sync/daemon.token (ipc.rs require_token:259, constant-time compare). The Linux adapter fails closed: secure_sync_directory enforces the app dir and sync dir are user-owned mode 0700 (ipc_storage.rs:170-190) and the token file is a single-link, uid-owned mode-0600 regular file (validate_token_identity:274-283, enforce_token_mode). The Windows adapter only checks that the token/addr files are regular, single-link and not reparse points (validate_private_file_handle:200-223) and that the directory is not a link (validate_directory:118-127); it never checks the owner SID or the DACL. Confidentiality of the token on Windows therefore relies entirely on the inherited ACL of %APPDATA% (support_dirs.rs:67), which the app never verifies.

**Fehlerszenario.** On a Windows machine where %APPDATA%\smart_explorer is created or redirected with permissive ACLs (shared/misconfigured profile, a prior tool that loosened inheritance, a redirected AppData on a shared volume), a second local user or a lower-integrity process reads daemon.token and daemon.ipc, connects to 127.0.0.1:<port>, and issues OpenShare (read/write the user's paired-peer files), ExecShare/ExecStream (run commands on peers that granted Exec), MutateExecGrant and StartMount - full Share control. The Linux build refuses to even use a token file it does not own at 0600, so the same misconfiguration fails closed there.

**Richtung.** On Windows, verify the token/addr/generation files and their parent directory are owned by the current user SID and carry a DACL granting only that user (GetSecurityInfo/GetNamedSecurityInfo), and create the sync dir with an explicit restrictive DACL rather than relying on %APPDATA% inheritance - mirroring the Linux fail-closed behavior.

**Gegenprüfung.** THE CODE FACTS ARE CORRECT.
- Windows sync_data_directory/validate_directory (windows/ipc_storage.rs:109-127) only reject reparse points and non-directories.
- create_token (:129-147) uses create_new with the inherited ACL and no security attributes.
- validate_private_file_handle (:200-223) checks only nNumberOfLinks==1 and the directory/reparse attributes.
- An rg over native/src finds no GetSecurityInfo, SetSecurityInfo, owner-SID or DACL code at all.
- Linux, by contrast, chmods and uid-checks both directories (linux_os/ipc_storage.rs:170-190) and enforces a uid-owned, single-link 0600 token (:262-283).
- The token is the sole gate for all daemon-token commands (ipc.rs:198-201, 259-268).

THE FAILURE SCENARIO NEEDS A PRE-EXISTING MISCONFIGURATION. %APPDATA% (support_dirs.rs:64-70) inherits the user-profile DACL (user, SYSTEM, Administrators), so another standard user cannot read daemon.token by default. Default folder redirection and roaming setups also grant the user exclusive rights. If AppData really were readable by another user, everything else in it would be exposed too. This is a defense-in-depth gap: no owner/DACL verification, and no explicit owner-only DACL on creation.

The 'lower-integrity process' remark is technically true: files carry no No-Read-Up label by default, so a Low-IL process of the same user can read the file. But that reader is the same account, and Linux has no stronger guard against same-user readers either. Severity should be low, not medium.

## S60 Pre-auth connection slots are exhaustible by any local process/app that reaches the loopback port, stalling the Share control plane (local DoS)

- Schwere: medium · Urteil: confirmed · Kategorie: reliability · Plattformen: Linux, Windows, Android
- Stellen: native/src/daemon/os/shared/ipc_listener.rs:17-18; native/src/daemon/os/shared/ipc_listener.rs:126-156; native/src/daemon/os/shared/ipc_listener.rs:163-198; native/src/daemon/os/shared/ipc.rs:28-33; native/src/daemon/os/shared/ipc_client.rs:181-200; native/src/daemon/os/shared/ipc_client.rs:215-240; native/src/daemon/os/shared/ipc_client.rs:277-291; native/src/daemon/os/shared/ipc_client.rs:325-356; native/src/daemon/os/shared/handoff.rs:22-30; native/src/daemon/os/shared/handoff.rs:154-164

**Beschreibung.** admit_client (ipc_listener.rs:126) accepts any loopback peer (ip().is_loopback() check at :133) and takes one of MAX_PRE_AUTH_CONNECTIONS = 16 permits (:18, :136) before the token is ever read. The permit is held for the whole pre-auth read, bounded only by PRE_AUTH_READ_TIMEOUT = 5 s (:17, ipc.rs:29 read_pre_auth_line). A client that connects and sends nothing holds a permit for 5 s; 16 idle connections consume every slot and new clients are silently dropped (admit_client returns at :137 without serving). The listening port is not a secret at the OS layer - any local process can discover it via the kernel's socket table and connect, even without the token.

**Fehlerszenario.** Desktop multi-user: another logged-in user (who cannot read the 0700-protected port file but can enumerate loopback listeners) opens 16 connections that send no newline and reconnects every 5 s, starving the legitimate GUI poll (every 300 ms, share.rs:45) so Share status/open/exec all time out. Android: any other installed app holding INTERNET permission connects to 127.0.0.1:<ephemeral> of this app's embedded daemon (autostart DAEMON_IN_PROCESS on android) and holds all 16 slots, blocking the user's own Share control plane; the attacker still cannot authenticate (token is in app-private storage) but availability is denied.

**Richtung.** Reduce the pre-auth hold (lower timeout and read only a short bounded token line with a tight deadline), cap concurrent pre-auth connections per source address, and/or require the token within the first packet before allocating a long-lived slot. On Android consider binding an abstract/UID-restricted socket or adding a per-connection cost so one peer cannot hold all slots.

**Gegenprüfung.** THE MECHANICS ARE AS DESCRIBED.
- admit_client admits any loopback peer (ipc_listener.rs:133).
- It drops the stream unserved when no permit is free (:136-138). MAX_PRE_AUTH_CONNECTIONS=16 and PRE_AUTH_READ_TIMEOUT=5s (:17-18).
- The absolute deadline is set at :139 and enforced by read_pre_auth_line (:163-198).
- The permit is released only after authentication (ipc.rs:29-32). There is no per-peer or rate cap.

THE PORT IS NOT A BARRIER. Desktop users can read /proc/net/tcp or GetExtendedTcpTable, or simply scan loopback. On Android the daemon is embedded (autostart/mod.rs:15), and the facade still talks to it over loopback TCP (mobile/os/shared/pool.rs:159, domains/share_state.rs:432, share_settings.rs:106). Another app can scan 127.0.0.1 and hold the slots.

WORSE ON DESKTOP THAN DESCRIBED. Legitimate requests do not time out; they get EOF. probe_worker then returns Missing (ipc_client.rs:325-356), and ensure_worker_ready (:181-200) calls restart_worker_for_client (:215-240). launch_replacement (:277-291) spawns a new daemon and writes a handoff stop control (handoff.rs:22-30). That control retires the healthy worker (handoff.rs:154-164, Handoff(target) != generation). The GUI's 300 ms poll (app/core/share.rs:45) therefore turns the DoS into repeated daemon replacements that interrupt background sync jobs and live Share sessions. The attacker just rescans for the new port. On Android there is no restart (embedded.rs:45-51); calls simply fail.

Minor overstatement: Windows AppContainer processes cannot reach loopback by default. The impact is availability only and needs a hostile local principal (another user, or a malicious installed app), so medium is acceptable.

## S61 A single shared token grants any same-user process the full Share capability set, including remote command execution on paired peers

- Schwere: low · Urteil: partially_confirmed · Kategorie: security · Plattformen: Linux, Windows, Android
- Stellen: native/src/daemon/os/shared/ipc.rs:34-195; native/src/daemon/os/shared/ipc.rs:198-201; native/src/daemon/os/shared/ipc.rs:259-268; native/src/daemon/os/shared/ipc_protocol.rs:344-362; native/src/daemon/os/linux_os/ipc_storage.rs:130-190; native/src/daemon/os/linux_os/ipc_storage.rs:262-283

**Beschreibung.** The daemon exposes OpenShare (returns a full read/write BackendHandle to a paired peer's exported files, ipc.rs:81), ExecShare and ExecStream (run a program or shell on a peer that granted Exec, ipc.rs:100-117), MutateExecGrant (enable/disable Exec grants, ipc.rs:52), StartMount/MountHost* and AnalyzeShare. All of these are authorized by nothing more than the token file, which is readable by any process running as the same user (no per-operation consent, no binding to the GUI process, no peer-credential check - SO_PEERCRED is used only for the separate exec supervisor UNIX socket in share/os/linux_os/exec.rs, not for this loopback TCP IPC).

**Fehlerszenario.** Malware or any low-trust helper running under the user's account reads sync/daemon.token, connects to the daemon, and (a) reads/writes every file the user's Direct/Room peers export, and (b) issues ExecStream to any peer that granted this device Exec, obtaining remote code execution on those peers - all without the user interacting with Smart Explorer and without triggering any prompt.

**Richtung.** This is the designed same-user trust model, but given the Exec capability consider hardening: bind the token to the GUI process (e.g. hand it over a parent-child channel rather than a readable file), require an explicit user confirmation for first Exec from a new local client, and/or keep Exec grants off by default (verify EnableExec is opt-in). At minimum document that any same-user process inherits full Share+Exec rights.

**Gegenprüfung.** THE FACTS ARE ACCURATE. One bearer token, checked in require_request_auth -> require_token (ipc.rs:198-201, 259-268), authorizes every daemon-token request (ipc_protocol.rs:344-362). That includes OpenShare (ipc.rs:81-88), ExecShare (:100-103), ExecStream (:104-117), MutateExecGrant (:52-57) and StartMount (:130-133). There is no peer-credential check on the loopback TCP path.

THE TOKEN IS NOT THE WEAK LINK. Same-user code can read the 0600 token, but it can just as well read the secrets the daemon itself uses:
- On Linux, secrets-v1 records are same-uid 0600 (linux_file_store.rs:232-247) and hold the Iroh key and relation secrets in plaintext (#0).
- On Windows, Credential Manager entries are readable by any process of the same user (creds/os/windows.rs:50-56).
Such code can impersonate the device to the same peers, including ExecStream against peers that granted Exec, without the daemon. Where a real boundary exists, such as sandboxed same-user apps without home access, the token holds: it is 0600 in a 0700 directory (linux_os/ipc_storage.rs:130-190, 262-283).

ANDROID: REFUTED. The token is in app-private filesDir (InitConfig.kt:32, mobile/os/shared/init.rs:60-61). Other apps run under different UIDs. They can connect, but they cannot authenticate.

This is the inherent same-user trust model, not a boundary crossing: low or informational. A Unix socket with SO_PEERCRED, or a named pipe with an owner-only DACL, would be optional hardening.

## S62 Share identity and profile files are created world-readable (0644) and app_data_dir is created with default umask, exposing the peer graph until the daemon tightens the directory

- Schwere: low · Urteil: partially_confirmed · Kategorie: security · Plattformen: Linux
- Stellen: native/src/support_dirs.rs:89-93; native/src/share/os/shared/identity_store.rs:248-252; native/src/share/os/shared/profile_store.rs:373-377; native/src/daemon/os/linux_os/ipc_storage.rs:126-190; native/src/daemon/os/shared/ipc_client.rs:26-31; native/src/app/core/share.rs:115; native/src/app/core/init.rs:58

**Beschreibung.** support_dirs::app_data_dir() (support_dirs.rs:89) creates the directory with create_dir_all and no explicit mode, so on Linux it defaults to the umask (commonly 0755). save_identity (identity_store.rs:248) and save_profiles (profile_store.rs:373) stage their temp files with OpenOptions::new().write().create_new() and no mode, i.e. 0644. These files (share_identity.json, share_profiles.json) hold the device id, node id, public key, fingerprint, direct_lookup_id, contact list and room IDs - the user's whole peer graph (not the private secrets, which are in the 0700 secrets-v1 store). The directory is forced to 0700 only as a side effect of the daemon touching its token/addr (linux_os/ipc_storage.rs:179 enforce_directory_mode), not at creation.

**Fehlerszenario.** On a Linux system where the daemon has not yet run (or the GUI wrote identity/profiles first), another local user reads ~/.local/share/smart_explorer/share_profiles.json and learns every device the victim shares with, their node IDs and room IDs - a privacy/metadata disclosure. Even after the daemon tightens the directory, the files themselves remain 0644, so any later relaxation of the directory mode re-exposes them.

**Richtung.** Create app_data_dir with mode 0700 explicitly on Unix and write the identity/profile staging files at 0600 (set OpenOptionsExt mode), so confidentiality does not depend on the daemon having run. Connections.txt already lands at 0600 via tempfile; apply the same to identity/profile.

**Gegenprüfung.** CONFIRMED:
- app_data_dir uses create_dir_all with the default umask (support_dirs.rs:89-93).
- save_identity (identity_store.rs:248-252) and save_profiles (profile_store.rs:373-377) stage files with create_new and no mode, giving 0644 or 0664 under umask 022/002.
- They are promoted by a plain rename (vfs/core/promotion.rs:46-52 -> local.rs:151-153), so the mode is kept.

OVERSTATED:
1. Tightening is not daemon-only. Every IPC client read calls secure_sync_directory, which chmods the app dir to 0700 (linux_os/ipc_storage.rs:48-57, 114-132, 170-190). Examples: ipc_client.rs:26-31 and 160-161, ipc_share_client.rs:16-17. Enabling Share in the GUI writes the profiles and then immediately calls refresh_share_worker_checked (app/core/share.rs:100-115). So share_profiles.json is world-readable only for a short window.
2. share_identity.json is created at every GUI start (app/core/init.rs:58). It can stay 0644 in an untightened directory if Share and background sync are never used. But it holds only public identifiers (identity.rs:15-27): device id, name, lookup id, public key, fingerprint, node id. The Direct code also needs the secret (profiles.rs:197; SE-D3-lookup-secret-fp-node).
3. Exposure also needs a world-traversable $HOME/.local/share. Many current distros create 0700 or 0750 homes.
4. 'Later relaxation re-exposes' is speculative, because each IPC access re-applies 0700 (:179).

This is metadata disclosure in a narrow window. Low is right.

## S63 Default Share export seeds the user's entire home directory as a shared root for new Direct peers and Rooms

- Schwere: high · Urteil: confirmed · Kategorie: security · Plattformen: Linux, Windows, Android
- Stellen: native/src/share/core/profile_persistence.rs:63-70; native/src/share/core/profile_persistence.rs:231; native/src/share/os/shared/profile_operations.rs:138; native/src/share/core/session.rs:218; native/src/share/core/session.rs:252; native/src/share/core/fs.rs:17-27; native/src/share/core/fs.rs:311-344; native/src/share/core/server_fs.rs:118-258; native/src/share/core/signal_auth.rs:279-319; native/src/daemon/os/shared/ipc_host_events.rs:204-245; native/src/daemon/os/shared/ipc_host_events.rs:469-481; native/src/mobile/core/config.rs:84-98

**Beschreibung.** When a profile is first created with empty exports, profile_persistence.rs:63-70 seeds default_direct_exports with a single root labeled 'Home' pointing at the user's home directory (default_home). New Direct grants (profile_persistence.rs:231) and newly joined Rooms (profile_operations.rs:137, profile_persistence.rs:231) inherit this default_direct_exports. authorize_state then returns default_direct_exports for an accepted Direct peer and room.exports for a room member (session.rs:218, :252), i.e. read/write access rooted at the whole home directory. The server path (server_fs.rs) enforces root-confinement against that root but does not otherwise restrict it, and there is no read-only default.

**Fehlerszenario.** A user accepts a Direct peer or joins a Room expecting to share one folder; because the inherited default export is the entire home directory, the peer can browse and (via Write/WriteNew/Rename/Promote in server_fs.rs) modify everything under $HOME/%USERPROFILE%, including the app-data and dotfiles, unless the user noticed and narrowed the export. 'Secure by default' would scope or empty the default and make exports explicit and read-only until widened.

**Richtung.** Default to an empty export (explicit deny-all) or a narrowly scoped folder, require the user to add each shared root deliberately, and support a read-only export flag that is the default for newly accepted peers/rooms.

**Gegenprüfung.** CONFIRMED.
- First run seeds SharedRoot{'Home', home} (profile_persistence.rs:63-70). On desktop that is $HOME or %USERPROFILE% (app/core/init.rs:54-55, identity_store.rs:138-148); on Android it is the primary shared volume (InitConfig.kt:39, mobile/core/config.rs:88-98).
- Rooms copy it (profile_persistence.rs:231 is the room path; profile_operations.rs:138). Direct peers do not get a per-grant copy; they are served default_direct_exports directly at session.rs:218. Room members get room.exports at :252.
- SharedRoot and ShareExportConfig carry no permission field (fs.rs:17-27).
- server_fs.rs:118-258 serves Write, WriteNew, MkdirAll, CreateDir, Rename, Promote, CopyFile, RemoveFile and recursive RemoveDir with no write gate.
- Confinement is only 'under root' (fs.rs:311-344), and dot-directories are allowed (fs_paths.rs:14).
- Room members are auto-admitted by a valid room-secret HMAC (signal_auth.rs:279-319) and inserted unblocked (ipc_host_events.rs:204-245, 469-481). Anyone holding a room code gets the export without approval.

SEVERITY IS UNDERSTATED:
(a) On Linux the default root contains the plaintext credential store ($HOME/.local/share/smart_explorer/secrets-v1; support_dirs.rs:72-79, creds/os/linux_os.rs:13). An authorized peer can download the Iroh key, all Direct and Room secrets and the saved passwords. With those it can impersonate the device to its other peers, including peers that granted it Exec, and join its other rooms. On Windows the export includes %APPDATA%\smart_explorer.
(b) Write access to autostart locations gives code execution at next logon: the Startup folder under %APPDATA%, ~/.config/autostart, shell rc files. This sidesteps the default-deny Exec model (new members get ExecGrant::default(), ipc_host_events.rs:480; FILE_OPS_MATRIX.md:110-118).

The mobile code itself states the invariant the desktop default breaks: the default export must never be filesDir because 'credentials, tokens and keys live there' (mobile/core/config.rs:84-87). The UI only shows '1 Ordner (Home)' (share_helpers.rs:29-42). This contradicts the requested 'secure by default'.

Preconditions: Share is enabled, and a Direct peer was accepted or a room joined.

## S64 Peer RemoveDir triggers unbounded native recursion with no depth or entry budget: an authorized peer can crash the host daemon (stack overflow)

- Schwere: medium · Urteil: partially_confirmed · Kategorie: reliability · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/fs.rs:213-233; native/src/share/core/server_fs.rs:246-256; native/src/share/core/server_fs.rs:347-365; native/src/share/core/blocking.rs:16-23; native/src/share/core/blocking.rs:41-63; native/src/share/core/node.rs:125-131; native/src/vfs/os/linux_os/local_platform.rs:13-15; native/src/local_access/os/windows/paths.rs:18-35

**Beschreibung.** An FsRequest::RemoveDir from any authorized Direct peer or Room member is served by `remove_dir_recursive` (fs.rs:213), which is genuinely self-recursive (`remove_dir_recursive(be, &child)?` per subdirectory) with no depth limit and no entry/time budget. It runs via `control()` -> `blocking::run` -> `spawn_blocking` on Tokio's blocking pool, whose threads have the default ~2 MiB stack. The project's own local recursive delete (native/src/vfs/core/delete.rs:6-8) deliberately caps depth at 512 and entries at 1,000,000 for exactly this reason, but the Share host path does not reuse those guards. Because the default export is the whole home directory (already-known issue) and MkdirAll/CreateDir let a peer build an arbitrarily deep tree cheaply (each MkdirAll walks components iteratively, so creation never overflows), the peer can create a tree thousands of levels deep and then RemoveDir its root. Aggravator: a recursive delete of a huge tree is not a short operation yet it holds one of only 32 control-pool slots (blocking.rs:16) for its entire duration, starving browsing/stat/rename for all peers even without a crash.

**Fehlerszenario.** An accepted peer sends many CreateDir/MkdirAll requests to build /home/<user>/share/a/a/a/.../a nested ~20,000 levels deep (or one MkdirAll with a ~200k-char path for ~100k levels), then sends RemoveDir{path:"a"}. `remove_dir_recursive` recurses one stack frame per level, each frame holding a `Vec<VfsMeta>` from list_dir; the ~2 MiB blocking-thread stack overflows, which aborts the entire Smart Explorer daemon process (taking down Share for every peer and the local background worker). No grant, exec permission, or local access is required beyond an ordinary accepted Share relationship.

**Richtung.** Make the Share host deletion iterative (explicit work stack) or reuse the budgeted `vfs::core::delete` planner; enforce a depth cap and an entry/byte budget identical to the local delete path, and return an error when exceeded rather than recursing. Keep refusing symlink/reparse children (already done). Consider moving long recursive deletes off the 32-slot control pool onto an admission-slot path so one peer cannot starve metadata operations.

**Gegenprüfung.** Confirmed in the code: `remove_dir_recursive` (fs.rs:213-233) calls itself once per subdirectory (fs.rs:224) and has no depth, entry or time budget. RemoveDir reaches it through `simple` -> `control` -> `blocking::run` -> `spawn_blocking` (server_fs.rs:246-256, 358-363, 374-380; blocking.rs:41-63). The Share runtime is built without `thread_stack_size` (node.rs:125-131), so Tokio's default blocking-thread stack applies. The guarded local delete really does use caps (vfs/core/delete.rs:6-8, 132), and Share does not reuse them. Any accepted peer can reach this path: there is no read-only export mode (ShareExportConfig at fs.rs:23-27 has only roots and include_connections), and with no profile file the default Direct export is Home (profile_persistence.rs:63-70).

The crash is overstated for two of the three platforms. LocalBackend passes the full absolute path to every syscall (local.rs:76-78, 105-107, 170-172), and on Linux `to_os` is the identity (linux_os/local_platform.rs:13-15). Once the child path built at fs.rs:215 passes the kernel's PATH_MAX (4096 bytes), `stat`/`list_dir` return ENAMETOOLONG and the `?` at fs.rs:214/219 unwinds. So on Linux and Android the recursion stops at about (4096 - root length)/2 frames, however deep the on-disk tree is. That leaves a budget of roughly 1 KiB per frame inside the default stack. The frame holds an IntoIter, two VfsMeta values and a String, so it probably fits in release builds, but this cannot be settled without compiling. On Windows, paths are made verbatim (local_access/os/windows/paths.rs:18-35, plus std's long-path handling), which allows about 32K-character paths and thousands more levels. An overflow there is plausible and would abort the whole daemon process (the Share node lives in the daemon: ipc_host_service.rs:211). Connection-backed targets depend on the remote server's own path limits.

The 'arbitrarily deep tree' claim also does not hold on Linux: each MkdirAll is iterative (local.rs:230-252), but every request path is still limited by PATH_MAX.

Severity: the actor must already be an authorized peer with full write and delete access to the export. What this adds is availability loss (a crash of the Windows host daemon, which also runs syncs and mounts), so medium rather than high.

The slot-holding aggravator is real but not specific to RemoveDir. The control pool is one process-wide 32-permit semaphore (blocking.rs:16-23). One connection may have 64 concurrent bidi streams (keepalive.rs:8), so a single peer can occupy every control slot with any slow control operation, and other peers' browsing then waits on the async acquire (blocking.rs:46-49). Separately, the client gives up after PEER_OP_TIMEOUT of 60 s (peer_request.rs:305-324), while the host's `spawn_blocking` delete cannot be cancelled and keeps running.

## S65 Enabling "share saved connections" turns the host into an unrestricted read-write proxy into every credentialed remote server, for every peer

- Schwere: medium · Urteil: partially_confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/fs.rs:23-27; native/src/share/core/fs.rs:146-167; native/src/share/core/fs.rs:185-211; native/src/share/core/fs.rs:268-277; native/src/share/core/session.rs:218; native/src/share/core/session.rs:252; native/src/app/core/share_exports_ui.rs:36-52; native/src/app/core/share_exports_ui.rs:58-66; native/src/app/core/share_exports_ui.rs:151-158; native/src/connect/os/shared/connector.rs:358-385

**Beschreibung.** When `ShareExportConfig.include_connections` is true, the host advertises a synthetic "Verbindungen" mount and `resolve` (fs.rs:146-186) routes any peer path under it through `connection_mounts()` (fs.rs:268, which loads EVERY saved connection via `crate::creds::load_connections()`) and `resolve_connection` (fs.rs:184), opening the backend with the host's own stored secret (`crate::creds::get_secret_checked`). This exposes the host's SFTP / FTP / FTPS / WebDAV / SMB (Protocol::Share=UNC) servers to the peer using the host's credentials, as a confused deputy / lateral-movement path. The exposure is all-or-nothing (a single bool, see share_exports_ui.rs:58 and the "Alle gespeicherten Verbindungen" buttons) with no per-connection selection, the whole connection root is exposed with no sub-path confinement, and because Share has no read-only mode, the peer gets full write/delete/rename on those remote servers. The same `room.exports` / `default_direct_exports` drives it for both Direct peers and every Room member.

**Fehlerszenario.** The user turns on "Eigene gespeicherte Verbindungen freigeben" (or clicks "Gespeicherte Verbindung hinzufügen") to let one trusted device reach one SFTP server. Every Direct peer and every Room member can now browse into Verbindungen/<any saved server>, read every file on the user's production SFTP/WebDAV/SMB servers, and delete or overwrite files there, authenticating as the user via the host's stored credentials, for servers the peer has no account on. A single compromised or malicious room member thereby pivots into the host's entire remote infrastructure.

**Richtung.** Replace the single include_connections bool with an explicit per-connection allow-list, require an explicit read-only default for shared connections (default-secure, less-secure write access only as explicit opt-in per the user's requirement), and surface a clear warning that sharing a saved connection grants peers access with the host's credentials. Confine each shared connection to an optional sub-path like local roots.

**Gegenprüfung.** Confirmed mechanics:
- With `include_connections` set, `resolve` sends every 'Verbindungen/<name>' path to `connection_mounts()`, which maps every entry of `crate::creds::load_connections()` with no filter (fs.rs:154-166, 268-277).
- `resolve_connection` authenticates with the host's own stored secret: `get_secret_checked` plus `NetConnection::connect` for UNC (Protocol::Share), and `open_saved_at` for SFTP/FTP/FTPS/WebDAV/SMB, which also loads the stored secret (fs.rs:185-211; connector.rs:358-385).
- There is no read-only flag (fs.rs:23-27). Write, MkdirAll, Rename, RemoveFile and RemoveDir are served for any resolved target, connection targets included (server_fs.rs:118-256).
- The flag is one bool with no per-connection selection.
- The UI button labelled in the singular 'Gespeicherte Verbindung hinzufuegen' sets exactly the same global flag as 'Alle gespeicherten Verbindungen' (share_exports_ui.rs:151-158). A user who means to share one server therefore exposes all of them. This is the strongest part of the finding.

Overstated or inaccurate parts:
1. Reach is per export scope, not 'every Direct peer and every Room member'. The editor chooses either 'Alle Direktkontakte' or one room (share_exports_ui.rs:36-52, 4-33). Authorization returns `default_direct_exports` for Direct peers (session.rs:218) and that room's own `room.exports` for room members (session.rs:252). Enabling it for one room exposes nothing to Direct peers or other rooms. Enabling it for Direct does expose it to all accepted Direct peers, since Direct has no per-peer exports.
2. Paths are confined lexically to each saved connection's configured root: `split_clean` rejects '..' (fs_paths.rs:6-19), `join_under` is anchored at `c.root` (fs.rs:203), and UNC goes through `secure_local_target` (fs.rs:194). 'No sub-path confinement' really means no finer grain than the saved root. Server-side symlinks under a URL-protocol root are not confined (`open_saved_at` uses AgentFallback::Allow), but that is the same reach the stored account already has.
3. Platforms: only the desktop GUI can enable the feature. The Android UI and mobile facade have no include_connections toggle (no hits in android/app/src or native/src/mobile), and the default is false (fs.rs:23 derive Default; profiles.rs:73). So this is Windows/Linux. Android would honor an imported flag but offers no way to set it.
4. It is an explicit opt-in whose label says saved connections are shared, so 'confused deputy' is better described as a least-privilege and UI-honesty weakness: all-or-nothing, write access never disclosed, a misleading singular button.

The always-checked, disabled 'Share-Server-Verbindungen ausschliessen' box (share_exports_ui.rs:72-75) is ambiguous: UNC connections (Protocol::Share, as_str "share", creds/core/types.rs:9,20) are included and served (fs.rs:186-201). Medium is kept because of the misleading button and the undisclosed write access.

## S66 An accepted peer can delay its own revocation (and block all profile changes) by keeping reciprocal-repair streams in flight

- Schwere: medium · Urteil: confirmed · Kategorie: security · Plattformen: Windows, Linux, Android
- Stellen: native/src/share/core/node.rs:36-37; native/src/share/core/node.rs:213-222; native/src/share/core/server.rs:209-221; native/src/share/core/direct_reciprocal_transport.rs:191-251; native/src/share/core/configuration_runtime.rs:63; native/src/share/core/signal_commands.rs:445; native/src/daemon/os/shared/ipc_host.rs:264-285; native/src/daemon/os/shared/ipc_host_service.rs:183-189; native/src/daemon/os/shared/ipc_host_service.rs:251-262; native/src/share/core/session.rs:185-218

**Beschreibung.** Applying any profile/authorization change (ConfigureProfiles, Configure, set_direct_online, i.e. removing or blocking a peer) first calls `iroh.begin_runtime_transition()?` (configuration_runtime.rs:63, signal_commands.rs:445), which does `try_acquire_many_owned(RUNTIME_TRANSITION_PERMITS)` for ALL 8 permits (node.rs:217-222) and fails fast with WouldBlock if any permit is held. Each incoming reciprocal-repair stream acquires exactly one of those permits (direct_reciprocal_transport.rs:237-247) and holds it for the whole exchange (up to PEER_OP_TIMEOUT = 60 s), and the peer controls the pace of the hello/offer/commit/ack messages. On the embedded/Android daemon, `reload_inputs_locked` additionally defers applying the on-disk profile while `reciprocal_repair_in_flight()` is true (ipc_host.rs:264-285), so the removed peer stays authorized in the live node's auth state. Because the reciprocal stream is reached only after the connection-level PeerHello is authenticated against an Accepted grant, the peer that can do this is precisely the accepted peer the user is trying to remove.

**Fehlerszenario.** The user decides to remove a misbehaving accepted Direct peer. That peer keeps one (or up to 4) reciprocal-repair streams continuously open, stalling each protocol step near the 60 s deadline and immediately opening a new one. Every `begin_runtime_transition()` the removal needs returns WouldBlock, so the worker's configure fails (connected_fail_closed -> reconnect) and the embedded daemon defers the reload; the peer's grant remains active in the running node and it keeps reading/writing the exports for as long as it sustains the repairs. The same technique blocks all other config changes (adding exports, blocking room members) for every peer.

**Richtung.** Give revocation/configuration priority over opportunistic repair: e.g. let begin_runtime_transition signal pending repairs to abort and briefly block new repair admission, or cap/debounce incoming repairs per principal and refuse new repairs from a peer whose removal is pending. Do not let the embedded daemon defer applying an on-disk removal indefinitely because a repair is active; apply revocations to the live auth state even while a repair holds a transition slot.

**Gegenprüfung.** The mechanism holds in the code:
- `begin_runtime_transition` needs all 8 permits through `try_acquire_many_owned` and fails at once with WouldBlock (node.rs:37, 217-222).
- Every profile or authorization change starts with it: `RuntimeConfiguration::apply` (configuration_runtime.rs:63; it serves ConfigureProfiles, Configure and discovery commits) and `set_direct_online` (signal_commands.rs:445).
- A `Ctrl::DirectReciprocal` stream is dispatched before the per-stream `authorize` (server.rs:209-221) but on a connection already authenticated by `authenticate_incoming_session` (server.rs:117; session.rs:69-82).
- `admit_incoming_direct_repair` takes one of 4 global repair slots and one transition permit, then calls `authorize_direct_repair` (direct_reciprocal_transport.rs:226-248). That re-checks for an Accepted Direct grant, relation_kind "direct" and the requested capability (session.rs:116-162). On success it holds both permits in the runtime guard for the whole exchange.
- The only bound is the absolute deadline of PEER_OP_TIMEOUT, 60 s (direct_reciprocal_transport.rs:201, 210-215; io_deadline.rs:7). The receiver waits for the peer's Hello and Commit with no shorter per-message timeout (direct_reciprocal_transport.rs:155-175). Nothing limits a peer to one stream or adds a cooldown.
- QUIC keepalives (keepalive.rs:5-7) and stream activity keep the connection from idling out.
- While any permit is held, `reciprocal_repair_in_flight()` is true (service.rs:193-195; node.rs:213-215). The daemon then defers the whole reload (ipc_host.rs:264-285), skips the start/restart sync (ipc_host_service.rs:183-189), and turns `configure_service` into a no-op (ipc_host_service.rs:251-257).
- Meanwhile filesystem authorization keeps reading the stale in-memory `direct_grants` (session.rs:185-218), while the GUI is fed the persisted profile, which no longer contains the peer (ipc_host.rs:270-281). The user sees the peer removed while it keeps access.

Corrections:
(a) The deferral is not limited to the embedded/Android daemon. `ShareHost` is built in the shared run loop (run_loop.rs:86), and the deferral at ipc_host.rs:264 is not gated on `is_embedded`, so it applies to Windows, Linux and Android alike.
(b) Normally the configure is never sent. The WouldBlock -> `connected_fail_closed` -> reconnect path (signal_commands.rs:63-72, 112-127) occurs only when a repair starts between the in-flight check and `apply`.
(c) Room members cannot do this (session.rs:123-128). Permits taken before a failed authorization are dropped right away when the `?` at line 248 returns.
(d) There is a fallback: an explicit Share stop does not need the semaphore. `stop_locked` -> ShareCmd::Stop -> `stop_sharing` sets sharing inactive, invalidates every session and closes the endpoint (ipc_host_stop.rs:3-16; signal_commands.rs:240, 356; node.rs:342-352). A restart rebuilds the worker from the persisted profiles.

Medium fits: it needs a modified client of an already-accepted Direct peer, revocation is silently delayed behind a misleading UI, and a full Share stop still works.

## Widerlegt

- (local-surface-at-rest) MountHost IPC commands bypass the daemon token entirely, relying only on per-mount secrets passed through child-process environment: THE CITED LINES EXIST. ipc_protocol.rs:363-365 returns None for MountHost*, and ipc.rs:202-214 authorizes them through the per-mount token checks. The security claim does not hold.

LINUX: CONTRADICTED BY CODE. daemon/mod.rs:103-105 selects linux_os/mount_process.rs for linux/android, and its spawn (:10-19) always returns Unsupported ('nur unter Windows'). fail_start then clears the launch token (mount_manager.rs:200-221). No child exists and no token is ever placed in an environment, so the /proc/<pid>/environ scenario cannot happen.

WINDOWS: THE PER-MOUNT TOKENS ARE NARROWER CAPABILITIES, NOT A BYPASS.
- They authorize only Attach/Backend/Status for one mount id (mount_manager_host.rs:70-107).
- The launch token is consumed on first attach (:58).
- The backend token is one-shot and guarded by backend_stream_active (:143-158).
- The child deletes the variables at startup (mount_client.rs:231-237).
- The block is handed over via CreateProcessW to a same-user child (mount_launch.rs:47-48, 74-90).

Reading another process's environment requires the same user or an admin. That principal can already read daemon.token (user-inherited ACL, windows/ipc_storage.rs:96-147), call StartMount/StopMount/OpenShare directly, and read the saved backend credentials from Credential Manager. Winning the race yields only a BackendHandle the attacker could already obtain, plus a DoS of that one mount. No privilege boundary is crossed. Informational at most; optionally the token could be passed through an inherited pipe instead of the environment.

## Offene Fragen der Prüfer

- [transport-encryption] iroh 1.0.1 (used by the app) is not vendored and no local registry copy exists: the semantics of presets::Minimal (whether any address discovery/publishing service is enabled) and the home-relay choice among several configured relays (assumed latency-based) could not be verified.
- [transport-encryption] The app uses upstream iroh-relay 1.0.1 while only 1.0.0 is vendored; the http-without-TLS client behavior was confirmed in the vendored 1.0.0 client and assumed unchanged in 1.0.1.
- [transport-encryption] Windows UAC-bypass finding assumes Start-ScheduledTask by the unelevated user succeeds for the registered task; the code is designed around exactly that (uplink_helper.rs module comment), but it was not executed.
- [transport-encryption] Discovery/PIN pairing (OPAQUE, discovery_* files) is outside the read scope and was not reviewed; only noted that a per-offer start limit (MAX_PAIRING_STARTS_PER_OFFER_WINDOW = 12) exists.
- [transport-encryption] Whether the GUI/Android UI warns about the Home default export or plaintext transports was not checked (UI files outside scope except the server-address field).
- [transport-encryption] The tracked auto-accept rule was confirmed in the daemon (ipc_host_direct_events.rs:237); the exact policy filters in direct_auto_accept_denied were not reviewed.
- [transport-encryption] iroh/iroh-relay 1.0.1 sources are not present locally. The client's no-TLS rule for http relay URLs (finding 2) is inferred from the vendored iroh-relay 1.0.0 copy (vendor/iroh-relay-1.0.0/src/client/tls.rs:97-105). The latency-based home-relay choice (finding 1) is taken from docs.rs iroh net_report.rs, latest version.
- [transport-encryption] Finding 9 assumes Windows lets the unelevated daemon run the RunLevel-Highest task through Start-ScheduledTask. The daemon's own design relies on this (uplink_helper.rs:175-178), but I did not observe it at runtime.
- [transport-encryption] Finding 12: I did not check in iroh's source whether its direct addresses (the presence candidates) include every interface address. That determines how much the Hello's lan list adds.
- [transport-encryption] Finding 3 on Android: the real exposure depends on whether the app holds All-files access to /storage/emulated/0.
- [transport-encryption] Observation outside the findings: the firewall rule is re-created unconditionally and needs admin rights, so an unelevated Windows daemon will show a UAC prompt on every process start (share/os/windows/system.rs:15-55). This trains users to approve UAC prompts.
- [pairing-identity-rooms] No UI code was reviewed (out of scope), so it is unknown whether auto-accepted Direct grants, new room members or completed PIN pairings are surfaced prominently. If they are, the impact of findings 1, 3 and 4 is reduced; if not, they happen silently.
- [pairing-identity-rooms] It is unclear whether mutual-by-default Direct relations and inheriting default exports into rooms are intended product decisions. The code and docs/SHARE_SERVER.md describe the reciprocal repair as intended. Combined with the default Home read/write export, it still contradicts the 'secure by default' requirement.
- [pairing-identity-rooms] Deployment is unknown: which Share-server URL scheme (wss vs tcp/ws) is configured in practice, and whether the public server enforces TLS in front of the plain TCP listener.
- [pairing-identity-rooms] Not reviewed (out of scope): Exec authorization for room members and Direct grants (exec_auth.rs, exec_grant_runtime.rs), LAN presence (lan_presence.rs, which is unauthenticated by design and only gives dial hints), and possible TOCTOU between ensure_under_root and the later backend operation in fs.rs.
- [pairing-identity-rooms] Windows roaming (finding 13) depends on the domain having roaming profiles and on Credential Manager roaming policy. The persistence flag was read from keyring 3.6.3 in the local cargo registry, matching Cargo.lock.
- [pairing-identity-rooms] The remaining part of direct_reciprocal_wire.rs (decode helpers after line 320) was not read line by line; the encode path and frame limits were checked.
- [pairing-identity-rooms] Whether the daemon drops or re-queues a whole tracked-event group after a Permanent apply error (finding 14) was not followed into the event-queue retry policy.
- [pairing-identity-rooms] #15: whether iroh::SecretKey's Debug impl redacts is not verified; the iroh-base source is not vendored, only iroh-relay is.
- [pairing-identity-rooms] #2 (Android): exposure of the whole primary volume depends on the app holding all-files access (MANAGE_EXTERNAL_STORAGE); the permission/grant flow was not checked.
- [pairing-identity-rooms] #1: the publisher-side close reason for a connector that aborts at KE2 (CancelPairing → Cancelled) was inferred from the client code; the share-server cancel handler was not traced.
- [pairing-identity-rooms] #4 (IdentityReplaced): not verified whether the daemon re-queues tracked requests by itself after reset_outgoing_authorization_after_identity_replacement; this only affects how the half-broken state shows up.
- [pairing-identity-rooms] #14: whether the JSON files are readable by other users depends on $HOME permissions on the target distributions; not decidable from code.
- [pairing-identity-rooms] #3/#6/#11: how often clients republish presence and re-join rooms, which bounds how long a lookup or roster hijack lasts, was not measured.
- [authorization-host-access] Room membership source: session.rs authorize_state requires a room peer to be a non-blocked entry in RoomProfile.members with matching node_id/public_key AND to prove the room HMAC secret, but where members are populated (auto-registration from signed presence vs. explicit owner approval) lives in signal_session.rs/discovery_relation_store.rs, which are outside my read scope. If membership is auto-added on first authenticated presence, then possession of the room code (secret) alone grants room exports - confirm the intended room trust model and whether a 'secure by default' room should require owner approval before a new device gets exports.
- [authorization-host-access] Confinement TOCTOU on Local/UNC Dynamic exports is documented as not race-proof (mount_lease.rs:130-138 comment; fs.rs ensure_under_root re-canonicalizes per op but between check and open a component could be swapped). This matches the stated 'Unverified / requires trusted-root mode' model for mounts, but the Share default-export path serves untrusted-by-policy peers on the same Unverified Local backend without a trusted-root requirement; whether that residual race is acceptable for Share depends on the threat model for paired peers.
- [authorization-host-access] Secret storage at rest: share profiles/identity live under app_data (support_dirs) and the Linux credential FileStore uses mode 0600/0700 but is not encrypted at rest; this is outside this review dimension (authorization/transfer) but worth a dedicated check if at-rest protection of room/direct secrets is a requirement.
- [authorization-host-access] I could not run builds or tests per project rules, so all findings are from static reading; the resource-exhaustion findings (control pool, analysis slots, connection count) are substantiated from the code paths but their real-world impact depends on backend latency and tree sizes that I could not measure.
- [authorization-host-access] User point 1 (phone->PC analysis is slow and runs differently): the Android client's analyze.start for a Share location (native/src/mobile/os/shared/domains/analyze.rs:189-211) calls crate::analytics::scan_backend directly. That walks the host with a breadth-first, level-synchronized ListDir RPC per directory (analytics_backend.rs:11-39,152-190) and never uses the host-local StorageAnalysis worker that desktop clients reach through crate::analytics::scan_remote -> PeerBackend::scan_storage (remote.rs:6-20; backend.rs:227-233). This is the likely root cause of the reported slowness. It lies outside this finding set and needs a fix decision by the main agent, for example routing remote mobile analysis through scan_remote.
- [authorization-host-access] Connection::close semantics for #4 (immediate local close, pending writes fail with LocallyClosed, unsent stream data dropped) were taken from the Quinn/iroh library contract. The QUIC crate source is outside the read scope, so they were not verified against the vendored code.
- [authorization-host-access] For #5, I did not verify whether iroh/noq imposes any default cap on concurrent incoming connections at the endpoint level (library source out of scope). The application code imposes none.
- [authorization-host-access] For #0, I did not check whether the share profile JSON in the app data directory (inside the exported home on desktop) has any integrity protection. A peer with write access might edit persisted grants, including Exec policy. exec_grant_persistence.rs shows revision/CAS checks but no keyed integrity binding.
- [share-server] Not traced: whether a forged direct_offline or room_left also tears down Iroh sessions that are already established, or only blocks new dials through server routes.
- [share-server] Out of this dimension: legacy Direct contacts with an empty expected_node_id. verify_direct_presence pins only the fingerprint over public_key and never checks node_id == public_key (both come from the same Iroh key, identity.rs:452-462). Any other holder of the target's Direct code could therefore pin its own node ID through the first-presence TOFU. Needs the identity-pinning reviewer.
- [share-server] Not traced: whether reciprocal repair on the requester side persists a reciprocal grant before the target confirms, when access_state was forged to Accepted through the legacy decision path (direct_reciprocal.rs was not read).
- [share-server] Unclear whether the desktop runs a GUI-local ShareService and the daemon worker at the same time with the same device_id (two registrations per device). That would matter for the per-source limits and for room-entry flapping (join_room replaced_client).
- [share-server] UI not inspected: whether Exec grants for room devices are shown by the unauthenticated device_name.
- [share-server] Iroh internals not inspected: how the home relay is chosen when both an http:// and an https:// relay URL are derived from a mixed endpoint list.
- [share-server] Android background dimension: whether a phone in Doze can answer the relay's fixed 30 s pong window. Missing it drops the home-relay connection.
- [share-server] All findings are derived statically from the code. In line with AGENTS.md, nothing was built, run or tested, so the burst-limit arithmetic, the idle-outbox starvation and the denial-of-service thresholds have not been confirmed at runtime.
- [share-server] Lead outside the reviewed findings, traced from code only. On a desktop (not low power), every verified presence event marks the profiles changed (ipc_host_events.rs:86-119, 228-247). The daemon persists them and then always sends ConfigureProfiles (ipc_host_events.rs:338-380; ipc_host_service.rs:251-263). The worker answers every ConfigureProfiles with a full publish_all (signal_commands.rs:112-137); the skip for unchanged profiles exists only in low power (signal_session.rs:106-111). The republished presence has a new nonce, so peers receive it as a new direct_available or room_joined (tracked_direct.rs:240; state.rs:154-156) and do the same on their next daemon tick (~2 s, run_loop.rs:244-248). Two desktops that watch each other, or a room of desktops, may therefore republish their whole subscription set and rewrite the profile file on every tick, indefinitely. This would also make #10's bursts recur far more often than every 60 s, and every 750 ms during open_share's Refresh loop (ipc_host.rs:361-389). Needs a runtime check.
- [share-server] Related insider issue, not among the findings. Room presences are authenticated only with the shared room secret (signal_auth.rs:279-320). Any holder of a room code, including removed or locally blocked members, can sign a presence for another member's device_id with a different key. upsert_room_member then marks that member IdentityConflict and resets its Exec grant on every receiving device (ipc_host_events.rs:445-459).
- [share-server] Two items were traced from code but not executed: #1(d), where forged refreshes starve an idle phone's presence, and #5's assumption that every device holds one home-relay connection.
- [share-server] #11: whether Caddy (the documented TLS proxy) closes idle upgraded tunnels decides whether zombie connections persist behind it. With SE_SHARE_TRUSTED_PROXY_IPS set they then count only against the global 128 registrations.
- [share-server] #9: how much battery is lost depends on Android delivering socket data to the foreground-service process during Doze. The idle-keepalive design already relies on this (WakeKeeper.kt:8-14).
- [local-surface-at-rest] Whether EnableExec/exec grants are off by default and require explicit user action: I confirmed the IPC plumbing (MutateExecGrant, ExecShare, ExecStream) and that any token-bearing local client can invoke them, but did not read the GUI/CLI default-state code to confirm no peer has Exec unless the user opted in. If Exec defaults to on for accepted peers, finding 4 rises toward high.
- [local-surface-at-rest] Actual Windows ACL of %APPDATA%\smart_explorer in the field: finding 2 assumes default per-user Roaming ACLs protect the token; I could not verify the installer or any code path that might create/redirect that directory with broader ACLs.
- [local-surface-at-rest] Whether default_direct_exports is overridden to empty/narrow by the first-run setup UI before any peer is accepted: finding 7 is based on the persistence-layer default seeding home; the setup flow (native/src/app setup screens / cli/setup.rs) was out of my read scope and may already force the user to choose a narrower export.
- [local-surface-at-rest] Exact at-rest protection of Android app-private files against a rooted device or adb backup: data_extraction_rules disables cloud/device-transfer backup, but I did not confirm whether any secrets land outside filesDir (e.g. cache) where another path could expose them.
- [local-surface-at-rest] I could not run builds/tests (read-only, per AGENTS.md), so all findings are from static reading of the cited lines; none were dynamically exercised.
- [local-surface-at-rest] Product intent for Rooms (only the user's own devices vs. multi-person groups) materially affects #6 severity; not determinable from code.
- [local-surface-at-rest] Whether the GUI or Android UI shows the write scope and full path of the default 'Home' export at Direct-accept or room-join time was not fully reviewed (only share_helpers.rs export_summary was read).
- [local-surface-at-rest] For #2, port discovery on Android 10+ (/proc/net restrictions) was not verified empirically; the verdict relies on loopback port scanning, which needs no special permission.
- [local-surface-at-rest] For #4, the claim that Windows process objects carry a No-Read-Up mandatory label (so Low-IL code cannot read the mount host's environment) comes from platform knowledge, not from code in this repository.
- [local-surface-at-rest] No runtime reproduction was performed (read-only assignment); the DoS-driven daemon handoff churn in #2 is derived from static reading of ipc_client.rs and handoff.rs.
- [critic] Could not run builds/tests (AGENTS.md forbids local compilation), so the stack-overflow depth threshold for RemoveDir was reasoned from the ~2 MiB default blocking-thread stack and frame contents, not measured. The qualitative defect (unbounded recursion with no budget, unlike the budgeted local delete) is certain; the exact depth that crashes a given platform was not empirically confirmed.
- [critic] The exact client-side flow that lets a peer build a very deep tree (repeated CreateDir vs a single long MkdirAll path) was confirmed in principle from wire.rs limits (MAX_REQUEST_CTRL_FRAME=256 KiB) and mkdir_all_plain being iterative, but I did not trace a concrete malicious client; a benign transfer engine would not do this.
- [critic] For finding 3 I confirmed the permit arithmetic (1 held vs 8 required) and the deferral logic by reading the code, but did not reproduce the revocation-delay end to end; the precise maximum delay depends on how aggressively a peer can re-open repairs within the 4-slot / 60 s limits.
- [critic] Exec runs as the host process's user and the provider reports an `elevated` flag (exec.rs / linux_os/exec.rs:267, windows/exec.rs) that is informational only. If a deployment ever runs the daemon elevated (e.g. a root systemd unit or an elevated uplink helper), an authorized exec peer would get elevated code execution. I did not find a default-elevated daemon path, so this is noted as a deployment caveat rather than a confirmed defect.
- [critic] I did not fully audit the OPAQUE discovery-pairing crypto (discovery_pake.rs) for protocol-level soundness beyond confirming CSPRNG seeding, a fixed-by-design KSF salt, and Argon2id parameters; a dedicated cryptographic review of the OPAQUE exchange and its transcript binding is out of scope here.
- [critic] The share-server/ and vendored iroh-relay sources were only spot-checked for the proxy/trust-anchor angle already covered by listed findings; a deeper review of server-side Room routing state machines was not repeated to avoid duplicating the existing share-server issues.
- [critic] Out of scope but on the cited RemoveDir path, and possible data loss: on the Share host, RemoveDir always deletes recursively (fs.rs:213-233, server_fs.rs:246-256, wire.rs:366-368, contract undocumented). `Backend::remove_dir` is non-recursive elsewhere (LocalBackend uses std::fs::remove_dir at local.rs:170-172). Mirror deletion keeps changed children (sync_delete.rs:196-197), then removes parent directories with `remove_dir` and relies on a non-empty directory failing (sync_delete.rs:75-83, 205-217). When the destination is a Share peer, a child the sync deliberately kept, or one added after planning, would be deleted along with its parent. Other remove_dir callers may have the same expectation: daemon mount_proxy.rs:140, backend_delete.rs:75, backend_walk.rs:197, app/core/delete_actions.rs:232, mobile/os/shared/delete.rs:132, cli/tree_remove.rs:35. Not verified end to end; needs its own check.
- [critic] Finding #0 on Linux/Android: whether about (4096 - root)/2 frames of `remove_dir_recursive` fit in Tokio's default 2 MiB blocking stack depends on the compiled frame size. That cannot be decided statically, so it needs a measurement on a release build. On Windows, verbatim long paths allow far deeper recursion, so an overflow is plausible there.
- [critic] Finding #1: `resolve_connection` opens a backend for every request (fs.rs:185-211). Whether `crate::connect::open_saved_at` pools sessions was not checked. If it does not, peer browsing through a connection mount repeatedly authenticates to the user's remote servers, which could trigger rate limits or account lockout.
- [critic] UI honesty on the same export panel: the 'Symlinks ausserhalb der Freigabe blockieren' checkbox (share_exports_ui.rs:68-71) toggles a field used only by the UI (state.rs:456, init.rs:445) and has no effect on the host, which always blocks escapes (fs.rs:311-344). The disabled, always-checked 'Share-Server-Verbindungen ausschliessen' (share_exports_ui.rs:72-75) does not match the code, which includes and serves UNC (Protocol::Share) connections.
