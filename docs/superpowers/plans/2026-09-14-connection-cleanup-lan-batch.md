# Connection cleanup, Drive duplicates, LAN presence and uplink sharing — Implementation Plan

> **For agentic workers:** executed inline by the main agent under `AGENTS.md`
> (no subagents, no local builds/tests; every milestone's expected result is
> verified once by the single remote-CI task suite). Steps use checkbox syntax.

**Goal:** Deleting a connection removes everything it created and cannot be
undone by the peer; leftover authorizations become deletable; ★ favourites on
Direct shares open and show their remote; Google Drive listings with duplicate
names browse again; paired devices find each other over a router-less LAN with
zero clicks; one side shares its internet automatically after a one-time
authorization.

**Architecture:** Core/OS split per `AGENTS.md`: pure decisions in
`*/core/*.rs` (removal transaction, duplicate disambiguation, link
classification, uplink policy), OS facilities behind `*/os/*` adapters
(mDNS, IP Helper/`/proc`, ICS COM + scheduled task, NetworkManager D-Bus). The
daemon (`se --sync-daemon`) owns LAN presence and uplink orchestration; GUI and
CLI only render state and issue commands.

**Tech Stack:** Rust 2021, egui, iroh 1.0.1, mdns-sd 0.11.5, if-addrs 0.13,
zbus 5.16 (Linux), windows 0.58 / windows-sys 0.59 (Windows), serde_json.

**Spec:** `docs/superpowers/specs/2026-09-14-lan-direct-uplink-design.md`
(Stages 1–2). Milestones 1–3 are bounded repairs whose root causes are recorded
below.

## Global Constraints

- No `cargo` invocation on this workstation; static edits only; one remote suite.
- New/edited Rust files < 500 lines and < 50 KiB; oversized existing files
  (`app/core/share.rs` 1174 lines) receive one-line call-outs only, new logic
  goes into new submodules.
- `core/` stays platform-independent (no `cfg(windows)`, no OS crates).
- Recoverable failures are `Result`; no `unwrap`/`expect` in production paths.
- "Never assume availability": every OS facility is probed and its absence is
  reported with a reason (`Unavailable(String)`), never treated as success.
- Share profile schema bump 7 → 8 is additive (`serde(default)`).
- UI copy in German (ASCII umlaut spelling as in neighbouring files), docs in
  English.
- Commit each milestone with the `[task candidate]` suffix; no version bump,
  no release until the batch is complete and the remote suite passed.

---

## Root causes being repaired

| # | Symptom | Root cause |
|---|---|---|
| R1 | Removing a Direct peer leaves "Autorisierte Geräte", requires a hard worker restart, peer comes back | `remove_direct_contact_persisted` (`share/os/shared/profile_operations.rs:163`) leaves the `DirectGrant`; incoming access is authorized by grants only (`share/core/session.rs:184`); the peer's reciprocal repair re-creates the contact (`share/core/direct_reciprocal.rs:192`); a new access request from a device knowing our Direct code is auto-accepted (`daemon/os/shared/ipc_host_direct_events.rs:237`); the worker skips reloads while a repair is in flight (`daemon/os/shared/ipc_host.rs`) and the GUI replaces its state with the worker snapshot (`app/core/share_drain.rs:151`). |
| R2 | Inactive authorizations cannot be deleted; revoked device cannot be re-paired | Grants are only ever set to `Ignored`; no delete path; `PolicyDenied::GrantIgnored` blocks explicit pairing too. |
| R3 | ★ favourites on `share://direct/...` fail with "Verbindung nicht gefunden"; no remote prefix in the label | `navigate_to_location` (`app/core/prefs_tabs.rs:113`) routes through `parse_remote_url` which reads `share` as the UNC `Protocol::Share`; labels are basenames (`app/core/sidebar_locations.rs:84`). |
| R4 | Google Drive root fails: `backend returned duplicate child name in "/"` | Drive allows same-name siblings; `GDriveBackend::list_dir` returns raw names and every walker (`rscan/os/shared/walk_state.rs:231`, analytics, cli) rejects the listing. `find_child` picks Drive's first hit (arbitrary). |

---

## Milestone 1 — Google Drive duplicate names (R4)

**Files:**
- Create: `native/src/gdrive/core/duplicates.rs`
- Modify: `native/src/gdrive/core/backend.rs` (`list_dir`, `stat`), `native/src/gdrive/core/metadata.rs` (`find_child`, `validate_cached_id`), `native/src/gdrive/core/dedupe.rs` (raw listing), `native/src/gdrive/mod.rs` (module)

**Interfaces (produces):**
- `duplicates::MARKER_PREFIX = " [drive-id "`, `MARKER_SUFFIX = "]"`, `ID_PREFIX_LEN = 8`
- `duplicates::canonical_first(entries: &mut Vec<VfsMeta>)` — sorts each same-name group newest-first, tie by id
- `duplicates::disambiguate(entries: Vec<VfsMeta>) -> Vec<VfsMeta>` — the canonical entry keeps the plain name, every further sibling gets `"{name} [drive-id {id8}]"`
- `duplicates::parse_marker(name: &str) -> Option<(&str, &str)>` — `(plain name, id prefix)`
- `duplicates::select_child(name: &str, siblings: &[(String /*id*/, i64 /*mtime*/)]) -> Result<Option<String>, String>` — canonical or marker-addressed id, ambiguous prefix fails closed
- `GDriveBackend::list_dir_raw(&self, path) -> VfsResult<Vec<VfsMeta>>` (crate-private)

- [ ] `duplicates.rs` with unit tests: no duplicates unchanged; two folders → second gets marker; marker round-trips through `parse_marker`; `select_child` picks newest, marker by prefix, ambiguous prefix → `Err`; names containing a literal marker but no duplicate resolve literally.
- [ ] `list_dir` = `disambiguate(list_dir_raw)`; cache every returned (possibly marker) path → id as trusted; `remember_path` for both plain and marker names.
- [ ] `find_child`: query `name = '<plain>'` with `pageSize=100`, fields `id,modifiedTime`, choose via `select_child`.
- [ ] `validate_cached_id`: compare the Drive name with the plain name and, for marker keys, require `id.starts_with(prefix)`.
- [ ] `stat`: for marker paths return the marker name.
- [ ] `dedupe.rs` and bisync duplicate-file logic keep using raw names (`list_dir_raw`).

**Expected result:** a Drive root with two folders `X` lists `X` (newest) and
`X [drive-id ...]`; both open; sync/bisync see two distinct directories; the
duplicate-file dedupe plan is unchanged (tested through `select_file_candidates`
and a `list_dir_raw` fixture).

---

## Milestone 2 — Complete peer removal in the profile core (R1, R2)

**Files:**
- Create: `native/src/share/core/removed_direct_peers.rs`
- Modify: `native/src/share/core/profiles.rs` (field, schema 8), `native/src/share/core/profile_persistence.rs` (version gate), `native/src/share/core/direct_reciprocal.rs` (`PairingOrigin`), `native/src/share/core/legacy_direct_request.rs` (`direct_auto_accept_denied`), `native/src/share/os/shared/direct_reciprocal_persistence.rs` (origin parameter), `native/src/share/core/discovery_relation_store.rs` and `native/src/share/os/shared/discovery_relation_store_adapter.rs` (origin `UserPairing`), `native/src/share/os/shared/profile_operations.rs` (`forget_direct_peer_persisted`), `native/src/share/mod.rs` (exports)

**Interfaces (produces):**
- `pub struct RemovedDirectPeer { device_id, public_key, fingerprint, node_id, device_name, removed_at }`
- `ShareProfiles::removed_direct_peers: Vec<RemovedDirectPeer>` (`serde(default)`, cap `MAX_REMOVED_DIRECT_PEERS = 64`, oldest pruned)
- `ShareProfiles::record_removed_direct_peer(&mut self, peer: &DirectPeerIdentity, now)`
- `ShareProfiles::removed_direct_peer(&self, peer: &DirectPeerIdentity) -> Option<&RemovedDirectPeer>` (device_id + public_key match)
- `ShareProfiles::readmit_removed_direct_peer(&mut self, device_id) -> bool`
- `pub enum PairingOrigin { UserPairing, AutomaticRepair }`
- `apply_reciprocal_direct_peer(peer, new_contact_id, now, origin)` — `AutomaticRepair` + removed record → `PolicyDenied(PeerRemoved)`; `UserPairing` clears the record first
- `persist_reciprocal_direct_peer(default_home, peer, origin)`
- `ShareProfiles::forget_direct_peer_persisted(default_home, contact_id) -> Result<(Self, ForgottenPeer), String>` where `ForgottenPeer { contact_id, display_name, device_identity: Option<DirectPeerIdentity>, cleanup_warning: Option<String> }`
- `ShareProfiles::delete_direct_grant_persisted(default_home, device_id) -> Result<(Self, ProfileChange), String>` (grant + its requests removed, removed record written)
- `ShareProfiles::readmit_removed_direct_peer_persisted(default_home, device_id)`

- [ ] `removed_direct_peers.rs` with tests: record/lookup/readmit, cap pruning, identity match rules.
- [ ] `forget_direct_peer_persisted`: one `mutate_persisted` closure removing contact, grant(s) for the contact's `remote_device_id`, tracked requests of that identity (outgoing by `contact_id`, incoming by requester pins) with `HistoryDeleted` tombstones (prune first; `TombstoneFull` → skip tombstone but keep the removed record, log), legacy requests of that identity, then `record_removed_direct_peer`; secret deletion as today.
- [ ] Reciprocal hooks + auto-accept denial + tests: automatic repair against a removed peer fails closed; user pairing readmits and installs.
- [ ] Schema gate accepts 7 (migrate) and 8.

**Expected result:** after `forget_direct_peer_persisted`, `direct_contacts`,
`direct_grants`, `direct_requests`, `legacy_direct_requests` contain nothing for
that device; `removed_direct_peers` has one record; `apply_reciprocal_direct_peer(..., AutomaticRepair)` returns `PolicyDenied`; `direct_auto_accept_denied` is true; `apply_reciprocal_direct_peer(..., UserPairing)` installs and clears the record.

---

## Milestone 3 — GUI/CLI removal cleanup, grants, favourites (R1–R3)

**Files:**
- Create: `native/src/app/core/connection_cleanup.rs`, `native/src/app/core/location_labels.rs`, `native/src/app/core/share_removed_devices_ui.rs`, `native/src/app/core/share_lan_ui.rs` (placeholder tab wiring lands in M5)
- Modify: `native/src/app/core/share.rs` (call-outs), `share_lifecycle_ui.rs`, `share_lifecycle_view.rs`, `prefs_tabs.rs`, `sidebar_locations.rs`, `landing.rs`, `sidebar.rs`, `menus_sync.rs`, `share_drain.rs`, `state.rs`, `menus_sync_jobs.rs`, `cli/setup.rs`, `cli/share/grants.rs`, `daemon/os/shared/ipc_host.rs`, `daemon/os/shared/mount_manager.rs` (lookup by source), `app/os/shared/platform_helpers.rs` (dir_sort removal helper)

**Interfaces (produces):**
- `connection_cleanup::EndpointScope { prefixes: Vec<String>, mount_matcher: MountMatcher }` with constructors `for_direct_contact(id)`, `for_room(profile_id, room_id)`, `for_saved_connection(&SavedConnection)`
- `App::cleanup_after_connection_removal(&mut self, scope: &EndpointScope) -> CleanupReport { favorites_removed, dir_sort_removed, mounts_stopped, tabs_closed, orphaned_sync_jobs }`
- `location_labels::location_label(app: &App, key: &str) -> String` → `"<Remote> › <Ordner>"` for remote keys, basename for local
- `App::open_share_target_at(target, path: Option<String>)`; `state.share_opening_path: Option<String>`
- `syncjobs::Job::is_orphaned(&self, known_prefixes: &[String]) -> bool` (computed at render)
- CLI: `se share grants delete <selector>`, `se share grants readmit <device>`

- [ ] "Entfernen" (GUI) and `se connections remove-peer` call `forget_direct_peer_persisted` then cleanup; saved-connection and room removal call cleanup with their scope.
- [ ] Authorized devices: active → additional "Geraet entfernen"; inactive → "Eintrag loeschen"; new collapsed section "ENTFERNTE GERAETE (n)" with "Erneut zulassen".
- [ ] Favourites: `navigate_to_location` handles `share://` via `PeerOpenTarget::from_endpoint` → `open_share_target_at`; `share_drain` scans the requested path; labels via `location_label` in sidebar and landing tiles.
- [ ] Sync jobs referencing removed endpoints render an "verwaist" badge and tooltip in `menus_sync_jobs.rs`; never deleted.
- [ ] Worker: `reload_now_locked` reloads `state.profiles` from disk even while a repair is in flight (service configuration deferred).

**Expected result:** removing a Direct peer deletes its favourites/dir_sort
entries, stops+removes its mounts, closes its tabs, shows the removed device
under "Entfernte Geräte", and the worker snapshot never resurrects the contact;
a `share://direct/<id>/Docs` favourite opens the peer at `/Docs`, labelled
`<Peer> › Docs`.

---

## Milestone 4 — Stage 1 LAN presence

**Files:**
- Create: `native/src/net/core/link_facts.rs`, `native/src/net/os/windows/interfaces.rs`, `native/src/net/os/linux_os/interfaces.rs`, `native/src/share/core/lan_presence_match.rs`, `native/src/share/os/shared/lan_presence.rs`, `native/src/share/core/lan_settings.rs`, `native/src/daemon/os/shared/lan_runtime.rs`, `native/src/cli/share/lan.rs`
- Modify: `share/core/types.rs` (contact fields, events), `share/core/service.rs` (`endpoint_for_target`), `share/core/session.rs` (`%ifindex` parsing), `daemon/os/shared/ipc_host.rs` (+ `lan` runtime in `tick`), `daemon/os/shared/ipc_host_events.rs`, `daemon/os/shared/ipc_host_service.rs` (`share_service_requested`), `daemon/os/shared/ipc_protocol.rs` (`LanStatus` in snapshot), `app/core/share.rs` (tab 4), `app/core/share_lan_ui.rs`, `app/core/share_drain.rs` (auto-open on `LanPeerSeen`), `cli/share.rs`, `native/Cargo.toml` (windows-sys `Win32_NetworkManagement_IpHelper`, `Win32_Networking_WinSock`)

**Interfaces (produces):**
- `net::InterfaceFacts { name, index: u32, up, loopback, addrs: Vec<IpAddr>, has_gateway, dhcp_lease: Option<bool> }`, `net::LinkClass::{RouterLess, Uplink, Routed}`, `net::classify_links(facts: &[InterfaceFacts], peer_ifaces: &[u32], internet_ifaces: Option<&[u32]>) -> Vec<(InterfaceFacts, LinkClass)>`, `net::gather_interface_facts() -> Result<Vec<InterfaceFacts>, String>` (OS adapter)
- `share::lan::hashed_lan_id(node_id: &str) -> String`
- `share::lan::LanSighting { id, addrs: Vec<IpAddr>, p4: u16, p6: u16, uplink: bool, seen_at: i64 }`
- `share::lan::match_sighting(contacts: &[DirectContact], sighting: &LanSighting, local_ifaces: &[InterfaceFacts]) -> Option<(String /*contact_id*/, Vec<String> /*candidates*/)>`
- `share::LanPresence::start(announce: LanAnnouncement) -> Result<LanPresence, String>`; `LanPresence::events() -> &Receiver<LanEvent>`; `LanPresence::update_uplink(bool)`
- `share::LanSettings { presence_enabled, uplink_sharing_enabled, uplink_setup_done }` + `load()/save()`
- `ShareEvent::LanPeerSeen { contact_id, candidates, uplink }`, `ShareEvent::LanPeerLost { contact_id }`
- `LanStatus { presence: LanFacility, peers: Vec<LanPeerView>, links: Vec<LinkView>, uplink: UplinkView }` in `ShareWorkerSnapshot`

- [ ] `link_facts.rs` + tests (APIPA only → RouterLess; gateway → Routed/Uplink; peer iface never uplink; ICS address recorded stays RouterLess).
- [ ] OS interface adapters with explicit `Result` errors.
- [ ] `lan_presence.rs` (mdns-sd advertise/browse thread) → `LanEvent::{Seen(LanSighting), Lost(id), Error(String)}`; failure to start is an `Unavailable` status, retried every 60 s.
- [ ] `lan_presence_match.rs` + tests (hash match, accepted-only, v4/v6 candidate strings with scope).
- [ ] Daemon runtime: owns `LanPresence`, refreshes announcement with the Iroh ports after service start, converts sightings to `ShareEvent`s, expires evidence, exposes `LanStatus`.
- [ ] `endpoint_for_target` merge/synthesis; `endpoint_addr` scope parsing + tests.
- [ ] GUI "LAN" tab and `se share lan [--json]`.

**Expected result:** with the Share server unreachable, a paired peer on the
same L2 segment appears within 10 s as `LAN` in the Direct list, "Oeffnen"
connects over the link-local address; the LAN tab lists interfaces with their
class; missing mDNS is shown as `nicht verfuegbar: <Grund>`.

---

## Milestone 5 — Stage 2 automatic uplink sharing

**Files:**
- Create: `native/src/share/core/lan_uplink_policy.rs`, `native/src/net/os/windows/ics.rs`, `native/src/net/os/windows/uplink_helper.rs`, `native/src/net/os/linux_os/nm_shared.rs`, `native/src/net/os/linux_os/uplink_polkit.rs`, `native/src/net/core/uplink_state.rs`, `native/src/daemon/os/shared/lan_uplink_runtime.rs`
- Modify: `net/mod.rs`, `bin/se.rs` (`--lan-uplink-helper`), `daemon/os/shared/lan_runtime.rs`, `app/core/share_lan_ui.rs`, `cli/share/lan.rs`, `native/Cargo.toml` (windows feature `Win32_NetworkManagement_WindowsFirewall`, `Win32_System_Variant`, `Win32_System_Ole`), `docs/GOTCHAS.md`, `README.md`

**Interfaces (produces):**
- `UplinkPolicy::evaluate(&mut self, input: PolicyInput, now: i64) -> Decision` with `Decision::{Keep, Start { private_if, public_if }, Stop(String), Idle(String)}`
- `trait UplinkAdapter { fn probe(&self) -> Facility; fn setup_once(&self) -> Result<(), String>; fn enable(&self, private_if: &InterfaceFacts, public_if: &InterfaceFacts) -> Result<(), String>; fn disable(&self, private_if: &InterfaceFacts, public_if: &InterfaceFacts) -> Result<(), String>; fn sharing_active(&self, private_if) -> Result<Option<bool>, String> }` with `Facility::{Available, Unavailable(String)}`
- `net::uplink_adapter() -> Box<dyn UplinkAdapter>` (Windows ICS, Linux NM, otherwise `UnsupportedAdapter`)
- `UplinkState` file `{ sharing: Option<{private_if, public_if, since}>, last_error: Option<String> }`
- CLI `se share lan uplink enable|disable|status`

- [ ] Policy + tests (debounce, grace, both-have-internet, tie-break, disabled, adapter unavailable).
- [ ] Windows ICS COM + helper mode + task setup; Linux NM D-Bus + polkit rule setup; every probe returns `Unavailable(reason)` instead of assuming.
- [ ] Daemon runtime applies decisions, persists state, reconciles at start/stop, announces `up=` to peers.
- [ ] GUI toggle (runs setup, shows outcome) and status; CLI; docs (README feature paragraph, GOTCHAS security note, SHARE_SERVER.md LAN section, TODO board rows).

**Expected result:** on a router-less link with a paired peer, the device with an
uplink enables sharing within ~5 s after a one-time consent; the peer receives
DHCP and internet; unplugging stops sharing within 90 s; on a system without
NetworkManager/ICS the LAN tab shows `Internet-Teilen nicht verfuegbar: <Grund>`
while LAN presence keeps working.

---

## Milestone 6 — Task-level suite, docs, graph

- [ ] One suite entrypoint (`native/scripts/task_suite.sh` / existing pattern) running the focused tests of M1–M5 plus the affected integrations (`share`, `gdrive`, `net`, `app::core::prefs_tabs`, `daemon::ipc_host`).
- [ ] README (Teilen section: Entfernen, Entfernte Geraete, Favoriten-Labels, LAN, Internet-Teilen), `docs/SHARE_SERVER.md` LAN presence section, `docs/GOTCHAS.md`, `docs/TODO.md` rows (21e LAN presence, 21f uplink sharing, C1 connection cleanup, G1 Drive duplicates).
- [ ] Graph refresh must be run by a user with access to `graphify` (not available to this agent).

**Expected result:** remote suite green once; then the terminal release per `AGENTS.md`.
