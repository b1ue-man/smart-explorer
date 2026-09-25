# Android Share/P2P Facade Map

**Purpose.** Ground the API contract of the future `native/src/mobile/` Share facade by inventorying every GUI file under `native/src/app/core/share*.rs`, the `crate::share` core types/commands those files drive, the daemon IPC surface that carries commands to the background worker, the `share://` endpoint format, and the existing `se share` CLI as a second, egui-free reference implementation of the same nine user actions.

## Files read

- `native/src/app/core/share.rs` (362 lines — module hub + `ensure_share`/`share_cmd`/`open_share_target`/`commit_share_profiles`)
- `native/src/app/core/quickshare_ui.rs`
- `native/src/app/core/share_diagnostics_ui.rs`, `share_direct_ui.rs`, `share_discovery_events.rs`, `share_discovery_retention.rs`, `share_discovery_state.rs`, `share_discovery_ui.rs`, `share_drain.rs`, `share_exec_jobs_ui.rs`, `share_exec_ui.rs`, `share_exports_ui.rs`, `share_helpers.rs`, `share_identity_rotation.rs`, `share_lan_ui.rs`, `share_lan_uplink_ui.rs`, `share_legacy_lifecycle_ui.rs`, `share_lifecycle_ui.rs`, `share_lifecycle_view.rs`, `share_navigation.rs`, `share_poll_status.rs`, `share_profile_cache.rs`, `share_profile_edits.rs`, `share_removal_ui.rs`, `share_removed_devices_ui.rs`, `share_rooms_ui.rs`, `share_window_ui.rs`
- `native/src/share/mod.rs`
- `native/src/share/core/types.rs`, `profiles.rs`, `profile_persistence.rs`, `direct_ledger.rs`, `room_relation.rs`, `discovery_signal_types.rs`, `exec_types.rs`, `fs.rs`, `endpoint_routes.rs` (skim), `direct_request_tombstone.rs` (skim), `legacy_direct_request.rs`, `identity.rs`
- `native/src/share/os/shared/profile_store.rs`, `profile_operations.rs`, `direct_actions.rs`, `identity_store.rs`
- `native/src/daemon/os/shared/ipc_client.rs`, `ipc_host.rs`, `ipc_host_service.rs`, `ipc_protocol.rs`
- `native/src/connect/core/location.rs`
- `native/src/cli/share.rs`, `native/src/cli/share/identity_command.rs`, `status.rs`, `lan.rs`, `grants.rs`, `grants_exec.rs`, `grants_removed.rs`, `exports.rs`, `requests.rs`, `requests_inbox.rs`, `requests_legacy.rs`, `exec_status.rs`

---

## Findings

### A — `native/src/app/core/share*.rs`: egui coupling and movability

| File | Imports egui/eframe? | Key `App` fields/methods it depends on | Movable out as-is? |
|---|---|---|---|
| `share.rs` (mod hub) | Only transitively via `use super::prelude::*;` (line 11); no `ui.*` calls in the file itself | `share`, `share_manual_stop`, `share_server`, `share_status`, `share_identity`, `share_identity_error`, `share_profiles`, `share_profiles_error`, `share_device_draft`, `error_msg`, `share_opening`, `share_opening_origin`, `share_opening_path`, `share_open_rx`, `remote`, `root_path`, `net_conn`, `scan_running`, `notice`, `share_diag_log`, `share_last_op_log_at`; calls `dirs_home()`, `self.start_scan(..)` | **Yes.** `ensure_share` (50‑131), `share_cmd` (133‑173), `configure_share_service` (175‑187), `commit_share_profiles` (189‑205), `open_share_target` (252‑301), `share_target_is_open`/`share_open_context_key`/`share_can_auto_open`/`mark_target_status` are pure orchestration over `crate::share`/`crate::daemon`. Needs the listed fields hoisted into a facade struct. |
| `quickshare_ui.rs` | Yes — `egui::Spinner`, `ui.button`, `RichText` | `show_share`, `quickshare`, `qs_devices`, `quickshare_error`, `share_device_draft`; calls `crate::quickshare::QuickShare::start`, `push_app_error` | Split: `drain_quickshare` (9‑36, polls an mDNS channel) is egui-free and movable; `ui_quickshare_devices` (38‑69) is pure rendering. Note: file transfer to Android Quick Share itself is **not implemented** (line 58 hover text) — LAN discovery only. |
| `share_helpers.rs` | Two functions take `egui::Ui`/`egui::Response` (`share_value_field`, `share_input_width`, 3‑18); rest is plain | reads `App.share_profiles`, `share_export_target_id` | Split: `selected_room_label`, `export_summary`, `share_open_result_is_current`, `trim_share_diag_log` (20‑68, with tests) are pure and movable verbatim. |
| `share_drain.rs` | No direct egui calls | `share`, `share_open_rx`, `share_opening(_origin/_path)`, `remote`, `net_conn`, `root_path`, `share_poll_rx`, `share_next_poll_at`, `share_profiles`, `share_worker_running/_relay_url/_candidates`, `share_lan_status`, `share_status`, `show_share`, `share_tab`; calls `mark_opening_status`, `share_target_is_open`, `cache_remote`, `start_scan`, `append_share_diag`, `apply_share_discovery_event`, `open_share_target`, `should_log_share_op` | **Yes**, this is `drain_share` (4‑366) — the entire poll/apply-events loop. `show_share=true; share_tab=0` (used as an "needs attention" signal, e.g. lines 174‑176, 213‑214, 235‑236, 301‑302) are plain fields, not egui state. |
| `share_navigation.rs` | No | `share_opening_path`, `share_target_is_open`, `start_scan`, `notice`, `open_share_target`, `share_opening` | **Yes**, `open_share_target_at` (7‑40). |
| `share_poll_status.rs` | No | none (pure `&str`/bool → `Option<&'static str>`) | **Yes**, verbatim, with its own unit tests. |
| `share_profile_cache.rs` | No | `share_identity`, `share_profiles`, `share_profiles_error`, `show_share`, `share_tab` | **Yes**, `reload` (3‑25). |
| `share_profile_edits.rs` | No | none — pure `ShareProfiles` three-way merge | **Yes**, verbatim (`merge_user_edits`/`merge_contact`/`merge_room`, 6‑112, with tests). This is the conflict-resolution logic the facade must reuse or re-derive exactly (see Facade recipes, "profile commits"). |
| `share_identity_rotation.rs` | No | `share_worker_running`, `share_identity`, `share_regenerate_direct_confirm`, `share_profiles`, `share_profiles_error`, `error_msg`, `notice`; calls `crate::daemon::is_running/drain_share_worker_events/send_share_command/refresh_share_worker_checked`, `dirs_home()` | **Yes**, `rotate`/`stop_worker`/`restore_worker`/`require_cleanup_before_restart` (9‑110). |
| `share_window_ui.rs` | Yes — `egui::Window`, `ScrollArea`, `RichText` | `show_share`, `share_tab` | No — pure tab-routing chrome; becomes screen navigation in the mobile app shell. |
| `share_direct_ui.rs` | Yes, throughout | reads/writes `share_profiles.direct_contacts`, `share_export_*`, `share_direct_code_input/_name_input`, `share_regenerate_direct_confirm`, `share_identity` | No (rendering), but the button handlers inline the exact calls the facade needs: `ShareProfiles::add_direct_from_code_persisted` (139‑152), `commit_share_profiles` (240), `remove_direct_peer_completely` (241‑243), `lifecycle_ui::queue_contact` (148, 245), `open_share_target` (247‑249), `identity_rotation::rotate` (104). |
| `share_rooms_ui.rs` | Yes, throughout | `share_profiles.rooms`, `share_room_*` inputs | No (rendering); inline calls: `ShareProfiles::new_room_code`/`add_room_from_code_persisted` (5‑11, 38‑50, 83‑96), `ShareProfiles::room_code_checked` (149‑155), `commit_share_profiles` (219), `share_cmd(LeaveRoom)` (221), `remove_room_completely` (223‑225), `open_share_target(RoomDevice)` (226‑228). |
| `share_exports_ui.rs` | Yes (rendering) + `rfd::FileDialog` (desktop-only picker, 130) | `share_profiles.default_direct_exports`/`rooms[i].exports`, `share_export_scope/_target_id/_label_draft/_path_draft`, `drives`, `root_path`, `remote` | `selected_export_config`/`set_selected_export_config` (4‑33) are movable as-is (pure `ShareExportConfig` get/set + `commit_share_profiles`); the folder-picker UI must be replaced by Android's Storage Access Framework, not reused. |
| `share_diagnostics_ui.rs` | Yes (rendering only) | `share_status`, `share_worker_running/_relay_url/_candidates`, `share_identity(_error)`, `share_diag_log` | No; adds nothing beyond calling `share_cmd(Refresh)`/`ensure_share()`, already covered by `share.rs`. |
| `share_removal_ui.rs` | `use super::*` in scope but **zero** `ui.*` calls in the file | `share_profiles`, `notice`, `error_msg`; calls `ShareProfiles::forget_direct_peer_persisted`/`delete_direct_grant_persisted`/`readmit_removed_direct_peer_persisted`/`remove_room_persisted`, `self.cleanup_after_removal` (in `crate::app::connection_cleanup`, **out of this reading's scope**), `configure_share_service` | **Yes**, `remove_direct_peer_completely`/`delete_direct_grant_entry`/`readmit_removed_device`/`remove_room_completely`/`remove_saved_connection_completely` (31‑186) — the canonical "remove X completely" recipes. |
| `share_removed_devices_ui.rs` | Yes (rendering) | `share_profiles.removed_direct_peers`; calls `app.readmit_removed_device` | No; trivial call-through, already covered above. |
| `share_discovery_state.rs` | **No egui at all** | none (standalone `DiscoveryUiState`/`DiscoveryPinDraft` state machine) | **Yes**, verbatim — the cleanest reusable piece in this surface (1‑478). `DiscoveryPinDraft` zeroizes its PIN buffer on drop (440‑478). |
| `share_discovery_events.rs` | Only `egui::Context` for `request_repaint()` (a no-op-able parameter) | `share_discovery` (`DiscoveryUiState`); calls `crate::daemon::send_share_command` | **Yes** with a trivial signature change (drop/replace the `repaint` parameter). `dispatch_discovery_ui_action` (124‑229) and `apply_share_discovery_event` (254‑339) are the authoritative client-side validation + event-folding logic. |
| `share_discovery_retention.rs` | No | none | **Yes**, verbatim (3‑53). |
| `share_discovery_ui.rs` | Yes, throughout | `share_discovery` (`DiscoveryUiState`), `share_profiles.rooms` | No (rendering); it builds `DiscoveryUiAction`s and defers to `dispatch_discovery_ui_action`, so its own PIN/duration checks are redundant with (and secondary to) the ones already in `share_discovery_events.rs`. |
| `share_lifecycle_ui.rs` | Yes for `ui_lifecycle`/`request_card`/`authorized_card` (37‑322) | `share_profiles`, `share_identity`, `error_msg`, `notice`, `share_next_poll_at`, `share_export_*`, `share_tab` | Split: `queue_contact` (104‑128), `perform_action`/`decide`/`retry`/`delete_history`/`refresh_after_action` (324‑470) are pure orchestration (`crate::share::queue_direct_request_for_contact/decide_direct_request/retry_direct_request_now/delete_direct_request_history/revoke_legacy_direct_request` + `profile_cache::reload` + `configure_share_service`) and are movable as-is. |
| `share_lifecycle_view.rs` | **No egui, no `App`** | none — pure `&ShareProfiles` → `RequestView`/`AuthorizedDeviceView` | **Yes**, verbatim (1‑472) — the read-model the facade should reuse for a requests/authorized-devices screen. |
| `share_legacy_lifecycle_ui.rs` | Yes for `ui`/`card` (15‑173) | `share_profiles.legacy_direct_requests`, `share_identity` | Split: `perform` (176‑222) is movable as-is (`decide_legacy_direct_request`/`retry_legacy_direct_answer`/`revoke_legacy_direct_request`/`delete_legacy_direct_request` + reload + refresh). |
| `share_lan_ui.rs` | Yes for `ui` (6‑105) | `share_lan_status`, `share_lan_notice` | Split: `set_lan_presence_enabled` (107‑121) movable as-is (`LanSettings::update` + `configure_share_service`). |
| `share_lan_uplink_ui.rs` | Yes for `ui` (6‑56) | `share_lan_notice` | Split: `set_lan_uplink_sharing_enabled`/`stop_lan_uplink_sharing_now` (58‑93) movable as-is. |
| `share_exec_ui.rs` | Yes for card/activation UI, incl. a two-step `ui.data_mut` "armed"/"understood" confirm gate (112‑174, egui-specific pattern) | none beyond `App` passthrough | Split: `apply_exec_grant` (210‑237, calls `crate::daemon::mutate_exec_grant`), `exec_device_views`/`exec_warning`/`display_name` (239‑312) are pure and movable; the two-tap confirmation must be reimplemented as a native Android confirm dialog. |
| `share_exec_jobs_ui.rs` | `egui::Id`/`ui.data_mut` cache + `egui::Context` for repaint, otherwise plain `Arc<Mutex<..>>` + `std::thread::spawn` | none beyond the cache | Split: `request_refresh`/`request_cancel` (209‑292, call `crate::daemon::exec_jobs`/`crate::daemon::cancel_exec`) are movable with the cache/repaint plumbing swapped for a mobile equivalent; `job_card`/`active_jobs`/`history_jobs` are pure rendering. |

### B — `crate::share` commands and events the GUI drives

`ShareCmd` (native/src/share/core/types.rs:345‑389) — sent via `crate::daemon::send_share_command`:

```rust
pub enum ShareCmd {
    ConfigureProfiles { profiles: Box<ShareProfiles> },      // durable-mutation only, rejected over IPC
    Configure { direct: Vec<DirectContact>, direct_grants: Vec<DirectGrant>, rooms: Vec<RoomProfile>, default_direct_exports: ShareExportConfig },
    SyncDirectRequests { direct_requests: Vec<DirectRequestEntry>, direct_request_tombstones: Vec<DirectRequestTombstone> },
    Refresh,
    Stop,
    SetDirectOnline { online: bool },
    EnableExec { target: ExecGrantTarget },                  // durable-mutation only, rejected over IPC
    DisableExec { target: ExecGrantTarget },                 // durable-mutation only, rejected over IPC
    ApplyExecGrant { target: ExecGrantTarget, principal: Box<ExecPrincipal>, policy: ExecGrant }, // durable-mutation only, rejected over IPC
    LeaveRoom { room_id: String },
    RequestDirect { contact_id: String },
    AnswerLegacyDirectRequest { selector: String, decision_revision: u64, lookup_id: String, requester_device_id: String, accepted: bool },
    Discovery(DiscoveryCommand),
}
```

`ShareEvent` (types.rs:399‑448), delivered inside `ShareWorkerSnapshot.events` from `drain_share_worker_events()`: `Status(String)`, `Error(String)`, `ServerConnected`, `ServerDisconnected(String)`, `DirectSignal(DirectSignalEvent)`, `DirectAvailable{lookup_id,presence}`, `DirectOffline{lookup_id}`, `DirectAccessRequest{lookup_id,presence}`, `DirectAccessAccepted{lookup_id,requester_device_id,accepted,presence,msg}`, `RoomRoster{room_id,members}`, `RoomJoined{room_id,presence}`, `RoomLeft{room_id,device_id}`, `Discovery(DiscoveryEvent)`, `RuntimeProfilesCommitted`, `LanPeerSeen{contact_id,candidates,uplink}`, `LanPeerLost{contact_id}`.

`DiscoveryCommand`/`DiscoveryEvent` (native/src/share/core/discovery_signal_types.rs:154‑257), sent/received wrapped in `ShareCmd::Discovery`/`ShareEvent::Discovery`:

```rust
pub enum DiscoveryCommand {
    Publish { target: DiscoveryPublishTarget, display_alias: String, pin: DiscoveryPin, duration_secs: u64 },
    StopPublishing { offer_id: String },
    ListDiscoveries,
    StartDiscoveryExchange { discovery_id: String, pin: DiscoveryPin },
    CancelDiscoveryExchange { exchange_id: String },
}
pub enum DiscoveryEvent {
    OfferPrepared { offer_id, target, display_alias, discoverable_until: i64 },
    OfferPublished { offer_id, target, display_alias, discoverable_until: i64 },
    OfferStopped { offer_id, reason: DiscoveryOfferStopReason },
    DiscoveryList { advertisements: Vec<DiscoveryAdvertisement> },
    ExchangeStarted { exchange_id, discovery_id },
    ExchangeCompleted { exchange_id, discovery_id, outcome: DiscoveryRelationOutcome },
    ExchangeCancelled { exchange_id, discovery_id: Option<String> },
    ExchangeFailed { exchange_id: Option<String>, discovery_id: Option<String>, error: String },
}
```

`DISCOVERY_PIN_MAX_BYTES = 1024` (discovery_signal_types.rs:9). `DiscoveryPin` wraps exact UTF‑8 bytes, zeroizes on drop, never trimmed/normalized (93‑150).

### C — Core data types the facade must model

| Type | Key fields | Source |
|---|---|---|
| `ShareProfiles` | `schema_version`, `auto_connect`, `default_direct_exports: ShareExportConfig`, `direct_contacts: Vec<DirectContact>`, `direct_grants: Vec<DirectGrant>`, `direct_requests: Vec<DirectRequestEntry>`, `legacy_direct_requests: Vec<LegacyDirectRequestEntry>`, `removed_direct_peers: Vec<RemovedDirectPeer>`, `rooms: Vec<RoomProfile>` | `native/src/share/core/profiles.rs:37‑65` |
| `DirectContact` | `id`, `display_name`, `lookup_id`, `expected_fingerprint`, `expected_node_id`, `remote_device_id/_public_key`, `auto_connect`, `auto_open`, `last_seen`, `status: ShareStatus`, `last_error`, `presence: Option<PeerPresence>`, `access_state: DirectAccessState`, `request_sent_at`, `accepted_at`, `accepted_public_key`, `lan_candidates`, `lan_seen_at`, `lan_uplink` | `types.rs:142‑174` |
| `RoomProfile` / `RoomMember` | `RoomProfile{id,name,room_id,auto_join,last_seen,status,members:Vec<RoomMember>,exports}`; `RoomMember{device_id,device_name,fingerprint,public_key,node_id,relay_url,candidates,last_seen,status,blocked,exec:ExecGrant,presence}` | `types.rs:176‑211` |
| `DirectRequestEntry` | `direction: DirectRequestDirection{Outgoing,Incoming}`, `contact_id`, `local_lookup_id`, `record: DirectRequestRecord`, `request_receipt`, `decision`, `decision_receipt`, `retries: DirectRequestRetries` | `native/src/share/core/direct_ledger.rs:69‑85` |
| `ShareExportConfig` / `SharedRoot` | `ShareExportConfig{roots: Vec<SharedRoot>, include_connections: bool}`; `SharedRoot{label,path}` | `native/src/share/core/fs.rs:17‑27` |
| `PeerOpenTarget` | `Direct{contact_id}` / `RoomDevice{room_id,device_id}`; `.endpoint_prefix() -> "share://direct/{id}"` or `"share://room/{room_id}/{device_id}"`; `::from_endpoint(&str) -> Option<(Self, path)>` | `types.rs:244‑311` (round-trip tests 313‑341) |
| `DiscoveryAdvertisement` / `DiscoveryKind` | `DiscoveryAdvertisement{discovery_id,offer_id,kind,display_alias,suite,version,expires_at}`, `.is_compatible()`; `DiscoveryKind::{Direct,Room}` | `discovery_signal_types.rs:13‑67` |
| `ExecJobView` | `exec_id: ExecId`, `peer_device_id`, `peer_device_name`, `program`, `command_digest`, `state: ExecLifecycleState`, `policy_revision`, `started_at`, `finished_at`, `terminal: Option<ExecTerminal>` | `native/src/share/core/exec_types.rs:186‑197` |
| `ExecLifecycleState` | `QueuedLocal, Connecting, Authenticating, Authorized, Starting, Running, Cancelling, Exited, Failed, TimedOut, Cancelled, Revoked, Disconnected` | `exec_types.rs:169‑183` |
| `ExecRequest` / `ExecResult` (blocking one-shot exec) | `ExecRequest{argv,cwd,timeout_ms,max_output_bytes,shell}`; `ExecResult{stdout,stderr,exit_code,timed_out,stdout_truncated,stderr_truncated}` | `native/src/share/core/types.rs:19‑35` |
| `LanStatus`/`LanSettings` | `LanStatus{presence: LanFacility, announced_id, peers: Vec<LanPeerView>, links: Vec<LinkView>, links_error, unknown_devices, uplink: UplinkView}`; `LanSettings{presence_enabled, uplink_sharing_enabled, uplink_setup_done, uplink_stop_requested_at}` | `native/src/share/core/lan_status.rs:29‑84`, `lan_settings.rs:6‑16` |
| `ShareWorkerSnapshot` (IPC reply) | `events: Vec<ShareEvent>`, `profiles: ShareProfiles`, `profile_revision`, `exec_grant_retry`, `running: bool`, `connected: bool`, `last_error`, `relay_url`, `candidates: Vec<String>`, `lan: LanStatus` | `native/src/daemon/os/shared/ipc_protocol.rs:193‑217` |

### D — Profile store load/commit functions (durable mutations, `native/src/share/os/shared/`)

| Function | Signature | Where |
|---|---|---|
| Load | `ShareProfiles::load_checked(default_home: Option<String>) -> Result<Self, String>` | `profile_store.rs:146‑148` |
| Generic commit | `ShareProfiles::mutate_persisted<F>(default_home: Option<String>, mutation: F) -> Result<ShareProfiles, String> where F: FnMut(&mut ShareProfiles) -> Result<(), String>` — optimistic compare-and-swap; `mutation` may re-run on conflict, must be idempotent/side-effect-free | `profile_store.rs:169‑182` |
| Add Direct by code | `ShareProfiles::add_direct_from_code_persisted(default_home, code: &str, name: &str) -> Result<(Self, String /*contact_id*/), String>` | `profile_operations.rs:19‑87` |
| Add/join Room by code | `ShareProfiles::add_room_from_code_persisted(default_home, code: &str, name: &str) -> Result<(Self, String /*room_profile_id*/), String>` | `profile_operations.rs:89‑97` |
| New room code | `ShareProfiles::new_room_code() -> Result<String, String>` → `"SE-R3-{room_id}-{secret_hex}"` | `profiles.rs:151‑157` |
| Room invite code (existing room) | `ShareProfiles::room_code_checked(room: &RoomProfile) -> Result<Option<String>, String>` | `profile_store.rs:235‑243` |
| Remove Direct completely | `ShareProfiles::forget_direct_peer_persisted(default_home, contact_id: &str) -> Result<(Self, ProfileChange, Option<ForgottenDirectPeer>), String>` | `profile_operations.rs:170‑193` |
| Delete leftover grant | `ShareProfiles::delete_direct_grant_persisted(default_home, device_id: &str) -> Result<(Self, ProfileChange), String>` | `profile_operations.rs:197‑214` |
| Readmit removed device | `ShareProfiles::readmit_removed_direct_peer_persisted(default_home, device_id: &str) -> Result<(Self, ProfileChange), String>` | `profile_operations.rs:217‑233` |
| Remove room completely | `ShareProfiles::remove_room_persisted(default_home, room_id: &str) -> Result<(Self, ProfileChange), String>` | `profile_operations.rs:235‑258` |
| `ProfileChange` | `{ changed: bool, cleanup_warning: Option<String> }` | `profile_persistence.rs:14‑17` |
| Direct/outgoing request lifecycle | `queue_direct_request_for_contact`, `decide_direct_request`, `retry_direct_request_now`, `delete_direct_request_history` | `direct_actions.rs:20,100,183,211` (signatures under Facade recipe 7) |
| Identity | `ShareIdentity::load_or_create(default_name: String) -> Result<Self,String>`; `.set_device_name(name: String) -> Result<(),String>`; `.regenerate_direct_code(&mut self) -> Result<DirectCodeRotation,String>`; `.complete_pending_cleanup(default_home) -> Result<ShareProfiles,String>` | `identity_store.rs:20,49,60,88` |

### E — Daemon IPC client surface (`native/src/daemon/os/shared/`)

| Function | Signature | Notes |
|---|---|---|
| `ensure_worker_ready` | `pub fn ensure_worker_ready() -> Result<(), String>` | Self-heals the daemon process (Ping → restart/handoff if `Missing`/`Stale`/stuck `Starting`/`Retiring`); called first by every other function below. `WORKER_READY_TIMEOUT = 25s`. `ipc_client.rs:178‑343` |
| `refresh_share_worker_checked` | `pub fn refresh_share_worker_checked() -> Result<bool, String>` — `Ok(true)` = a `ShareService` is running after the reload | `ipc_client.rs:112‑127`, IPC `RefreshShare` |
| `send_share_command` | `pub fn send_share_command(cmd: ShareCmd) -> Result<(), String>` | `ipc_client.rs:129‑143`, IPC `ShareCommand`; host rejects durable-mutation variants (see below) |
| `drain_share_worker_events` | `pub fn drain_share_worker_events() -> Result<ShareWorkerSnapshot, String>` — retries 3× / 250ms | `ipc_client.rs:145‑176`, IPC `DrainShareEvents` |
| `open_share_backend` | `pub fn open_share_backend(target: PeerOpenTarget) -> Result<(String /*label*/, BackendHandle, ShareStatus), String>` | `ipc_client.rs:19‑90`, IPC `OpenShare`; up to 8 retries with worker-restart-on-agent-error, host retries the connect for up to 45s (`ipc_host.rs:385‑414`) |
| `exec_share` (blocking one-shot) | `pub fn exec_share(target: PeerOpenTarget, req: ExecRequest) -> Result<ExecResult, String>` | `ipc_client.rs:92‑110`, IPC `ExecShare`; socket timeout = `max(req.timeout_ms + 60_000, 60_000)` ms |
| `mutate_exec_grant` | `pub(crate) use exec_grant_client::mutate_exec_grant;` re-exported at `ipc_client.rs:14`; called as `crate::daemon::mutate_exec_grant(target: ExecGrantTarget, enabled: bool) -> Result<ExecGrantPersistResult, String>` from call sites | Defining file `ipc_exec_grant_client.rs` **not in this reading's surface**; IPC `MutateExecGrant`/`ExecGrantMutation` shapes confirmed in `ipc_protocol.rs:233‑237,358‑360` |
| `request_daemon_replacement` | `pub fn request_daemon_replacement() -> Result<(), String>` | `ipc_client.rs:201‑203`, used after an update, not part of Share actions |

Host-side durable-mutation gate (`ShareHost::send_command`, `ipc_host.rs:315‑344`):

```rust
if matches!(&cmd, ShareCmd::EnableExec{..} | ShareCmd::DisableExec{..}
    | ShareCmd::ApplyExecGrant{..} | ShareCmd::ConfigureProfiles{..}) {
    return Err("Dieser Share-Befehl erfordert eine dauerhafte Daemon-Mutation".into());
}
```

`ShareCmd::Stop` is handled specially (`stop::stop_locked`); every other variant runs through `reload_now()` then `service.cmd(cmd)`.

`ShareHost::reload_now_locked` (`ipc_host.rs:208‑302`) is what the daemon does on every `RefreshShare`/periodic tick (every 5s, `tick()` at 129‑143): re-read `share_server.txt` and `ShareIdentity`, re-read `ShareProfiles::load_checked`, apply any pending Exec-grant journal recovery, then `configure_or_restart_locked` (`ipc_host_service.rs:3‑80`) starts/stops/reconfigures the live `ShareService` to match the freshly loaded profile and identity. **This is the only path that ever applies a profile edit to the live connection** — IPC commands never mutate `share_profiles.json` themselves (except the four rejected variants, which the daemon does apply directly in-process when it calls them internally, e.g. from `exec_grant_journal`).

Not read (outside the assigned four `ipc_*` files, needed for the full Exec-job facade — flagged in Unresolved): `native/src/daemon/os/shared/exec_ipc.rs`, `exec_state.rs`, `ipc_host_direct_events.rs` family, `ipc_exec_grant_client.rs`.

### F — `share://` endpoint format (`native/src/connect/core/location.rs`)

`EndpointSpec::parse(endpoint: &str) -> Result<Self, String>` (15‑51) recognizes a `scheme://rest` prefix and for `scheme == "share"` calls:

```rust
"share" => crate::share::PeerOpenTarget::from_endpoint(&format!("share://{rest}"))
    .map(|(target, path)| Self::Peer(target, path))
    .ok_or_else(|| "Ungültige Share-Adresse".into()),
```

So the wire format is exactly `PeerOpenTarget::endpoint_prefix()` (Finding C) plus an optional `/`-separated path suffix: `share://direct/{contact_id}[/sub/path]` or `share://room/{room_id}/{device_id}[/sub/path]`. `EndpointSpec::Peer(target, path)` is one arm of the same enum used for local/`gdrive://`/`sftp|ftp|ftps|webdav://` roots, so a Share endpoint is a first-class sibling of every other backend for overlap checks (`paths_overlap`, `validate_sync_endpoints`, 104‑157) and locator equality (`endpoint_key`, 116‑136, keys a Peer target by `target.endpoint_prefix()`).

The function that turns an `EndpointSpec::Peer` into an actually open connection (`resolve_endpoint`, presumably in `native/src/connect/os/shared/resolution.rs`) is **outside this reading's assigned surface** — flagged in Unresolved. Based on `location.rs` alone plus the GUI's own direct pattern (`open_share_target` → `crate::daemon::open_share_backend(target)`, Finding A), it almost certainly dispatches `EndpointSpec::Peer(target, path)` to `crate::daemon::open_share_backend(target)` and then navigates to `path`, mirroring what `share_navigation.rs`'s `open_share_target_at` does client-side.

### G — CLI (`native/src/cli/share/*.rs`) coverage of the nine actions

| Action | CLI reference | Gap vs. GUI |
|---|---|---|
| (1) Identity/device name | `identity_command.rs::run`/`load_with_repair_hint` (read + `--repair`); device name is set only via `se share configure --device-name` (`share.rs:62‑67,114‑134`) | none material |
| (2) Server config | `share.rs::configure`/`validate_server` (114‑134,179‑208) | GUI never calls the equivalent write path itself in this surface — see Unresolved |
| (3)/(4) Discovery (make discoverable, list, connect by PIN) | **No CLI subcommand exists** (`grep -rl Discovery native/src/cli/share` is empty) | Full gap — CLI cannot publish/list/pair via Discovery at all |
| (5) Direct devices list/open/remove | `status.rs` (list), `grants.rs`/`grants_removed.rs` (delete/readmit) | **Adding** a Direct contact by code has no CLI subcommand either (`add_direct_from_code_persisted` is never called under `cli/`) — only GUI and Discovery pairing create contacts |
| (6) Rooms create/join/list/leave/remove | `share.rs::create_room` (136‑147, `Create` only) | Join/list/leave/remove have **no CLI subcommand** (`RoomCommand` has only `Create`) |
| (7) Requests inbox/accept/reject/retry/delete | `requests.rs`, `requests_inbox.rs`, `requests_legacy.rs`, `request_selection.rs` | **Full parity** — best reference for this action |
| (8) Exports list/add/remove | `exports.rs` | **Full parity**, plus path canonicalization (`canonical_directory`, 167‑181) the GUI's text field lacks |
| (9) One-shot exec / jobs / grants | `exec_status.rs` (list/show/cancel/history), `grants_exec.rs` (enable/disable) | The **blocking** `exec_share`/`ExecResult` call has no CLI subcommand (`grep exec_share` under `cli/share` is empty); only the job-tracking (`ExecStream`) path is exposed |

---

## Facade call recipes

Each recipe assumes the facade owns its own state struct mirroring the `App` fields in Finding A (identity, profiles, worker status, diagnostics log) instead of `egui::App`, and that JSON commands map 1:1 onto the Rust calls below.

### 1. Show own device / set device name

1. `crate::share::ShareIdentity::load_or_create(default_name: String) -> Result<ShareIdentity, String>` (`identity_store.rs:20`). Persist: nothing (read, self-creating on first run). Fields to surface: `device_id`, `device_name`, `fingerprint`, `node_id`, `.direct_code() -> String`.
2. Worker/online status: `crate::daemon::drain_share_worker_events() -> Result<ShareWorkerSnapshot, String>` (`ipc_client.rs:145`). Surface `snapshot.running`, `snapshot.connected`, `snapshot.relay_url`, `snapshot.candidates`, `snapshot.lan.presence: LanFacility` for LAN status.
3. Configured share server: **unresolved** — no client-side reader of `share_server.txt` was in the assigned surface (only the daemon-side `load_share_server()`, `ipc_host.rs:440‑465`, and the CLI writer, `cli/share.rs:127‑130`, were read). The facade needs its own accessor (likely `crate::support_dirs::app_data_file("share_server.txt")` read directly, matching the CLI's write target) or a new IPC query.
4. Set device name: `identity.set_device_name(name: String) -> Result<(), String>` (`identity_store.rs:88`). Persist: writes `share_identity.json` + rotates nothing. Then call `crate::daemon::refresh_share_worker_checked()` (`ipc_client.rs:112`) so the daemon reloads identity and restarts `ShareService` if it changed (`ipc_host_service.rs:43‑55`). Await: just the `Result`; next poll (`drain_share_worker_events`) reflects the new name via `snapshot.profiles`/direct code.

### 2. Set/clear the share server address

1. Validate: `cli::share::validate_server(server: &str) -> Result<String, String>` (`share.rs:179‑208`) — non-empty, ≤16 KiB, no whitespace/`@`, scheme (if any) ∈ `{tcp,ws,wss,http,https}`.
2. Ensure `profiles.auto_connect == true` (persist via `ShareProfiles::mutate_persisted` if not, same as `share.rs:114‑126`).
3. Persist the server string atomically to `support_dirs::app_data_file("share_server.txt")` (see `write_atomic`, `cli/share.rs:210‑224`, for the exact staged-file pattern — not itself in the read surface as a shared helper, reimplement or share it).
4. Call `crate::daemon::refresh_share_worker_checked()`; error if it returns `Ok(false)`.
5. "Clear" is **not implemented anywhere in the read surface**: `validate_server` rejects an empty string, and the GUI never writes an empty `share_server.txt` — the GUI's "empty field" only skips calling step 3/4 (`ensure_share`, `share.rs:57‑61`). If the mobile app needs an explicit "forget server" action, it has no existing precedent to copy and must define its own semantics.

### 3. Make discoverable (Direct or Room) for N minutes with a PIN, stop, show countdown

1. Validate client-side (mirrors `share_discovery_events.rs:129‑166`): `duration_secs > 0`; `pin.as_bytes().len() <= DISCOVERY_PIN_MAX_BYTES` (1024).
2. `crate::daemon::send_share_command(ShareCmd::Discovery(DiscoveryCommand::Publish{ target: DiscoveryPublishTarget::Direct | DiscoveryPublishTarget::Room{room_profile_id}, display_alias, pin: DiscoveryPin::new(pin_string), duration_secs }))`. Persist: none (runtime-only offer).
3. Await via polling: `ShareEvent::Discovery(DiscoveryEvent::OfferPrepared{offer_id,..})` then `OfferPublished{offer_id,target,display_alias,discoverable_until}`. Countdown = `discoverable_until - now_unix`.
4. Stop: `send_share_command(ShareCmd::Discovery(DiscoveryCommand::StopPublishing{offer_id}))`; await `DiscoveryEvent::OfferStopped{offer_id,reason}` (`reason` ∈ `Requested,Expired,TargetUnavailable,CapabilityUnavailable,TransportError`).

### 4. List discoverable devices/rooms, connect with a PIN

1. `send_share_command(ShareCmd::Discovery(DiscoveryCommand::ListDiscoveries))`; await `DiscoveryEvent::DiscoveryList{advertisements: Vec<DiscoveryAdvertisement>}`. Fields: `discovery_id,offer_id,kind: DiscoveryKind,display_alias,suite,version,expires_at`; filter/annotate with `.is_compatible()`.
2. Connect: validate PIN length client-side, then `send_share_command(ShareCmd::Discovery(DiscoveryCommand::StartDiscoveryExchange{discovery_id, pin}))`.
3. Await: `DiscoveryEvent::ExchangeStarted{exchange_id,discovery_id}` (in-progress), then a terminal event — `ExchangeCompleted{exchange_id,discovery_id,outcome: DiscoveryRelationOutcome}` (success; the daemon has already durably installed the Direct contact or Room — just re-poll `drain_share_worker_events` and read the updated `profiles.direct_contacts`/`profiles.rooms`), `ExchangeFailed{..,error}`, or `ExchangeCancelled{..}`.
4. Cancel mid-flight: `send_share_command(ShareCmd::Discovery(DiscoveryCommand::CancelDiscoveryExchange{exchange_id}))`; await `ExchangeCancelled`.

### 5. Direct devices: list, build `share://` endpoint, open, remove completely

1. List: read `profiles.direct_contacts: Vec<DirectContact>` from the latest `ShareWorkerSnapshot.profiles` (or an offline `ShareProfiles::load_checked`). Online state = `.status: ShareStatus` (`Offline,Waiting,WaitingForAccess,Available,Connecting,Connected,ConnectedDirect,ConnectedRelay,Failed(String),IdentityConflict`).
2. Build endpoint: `crate::share::PeerOpenTarget::Direct{contact_id}.endpoint_prefix() -> "share://direct/{contact_id}"` (`types.rs:250‑258`); append `/sub/path` for a specific folder — `PeerOpenTarget::from_endpoint` round-trips this exactly (tests at `types.rs:317‑327`).
3. Open: `crate::daemon::open_share_backend(target: PeerOpenTarget) -> Result<(String, BackendHandle, ShareStatus), String>` (`ipc_client.rs:19`). This self-heals the daemon (`ensure_worker_ready`), and on the host retries the connection for up to 45s (`ipc_host.rs:385‑414`, `service.probe_backend_for_target`). The returned `BackendHandle` (`crate::vfs::Backend`) is then used for `list_dir`/`stat`/`open_read`/… to actually browse files — that VFS layer is outside this Share reading.
4. Remove completely: `ShareProfiles::forget_direct_peer_persisted(default_home, contact_id) -> Result<(ShareProfiles, ProfileChange, Option<ForgottenDirectPeer>), String>` (`profile_operations.rs:170`). Persist: deletes the contact, its relation secret, matching grant(s), matching requests, and records a `RemovedDirectPeer` denial so the device cannot silently re-pair. Then the same cleanup other endpoint removals need (favourites/mounts/open tabs — `crate::app::connection_cleanup`, out of this reading's scope) and `crate::daemon::refresh_share_worker_checked()`.

### 6. Rooms: create, invite code, join by code, list/members, leave, remove

1. Create: `ShareProfiles::new_room_code() -> Result<String, String>` (`profiles.rs:151`, format `SE-R3-{room_id}-{secret_hex}`), then `ShareProfiles::add_room_from_code_persisted(default_home, code, name) -> Result<(ShareProfiles, String /*room_profile_id*/), String>` (`profile_operations.rs:89`). Persist: room secret to secure storage + a new `RoomProfile{auto_join:true, status:Waiting, members:[], exports: default_direct_exports.clone()}`.
2. Show invite code (existing room): `ShareProfiles::room_code_checked(room: &RoomProfile) -> Result<Option<String>, String>` (`profile_store.rs:235`).
3. Join by code: same `add_room_from_code_persisted` call — idempotent if `room_id` already exists (`RoomPersistenceOutcome::AlreadyComplete`).
4. List/members: `profiles.rooms: Vec<RoomProfile>`, each with `.members: Vec<RoomMember>` (Finding C).
5. Leave: `send_share_command(ShareCmd::LeaveRoom{room_id})` (runtime signal) **and** persist `auto_join=false`/`status=Offline` on the local `RoomProfile` via `ShareProfiles::mutate_persisted` + the merge logic in Finding A/`share_profile_edits.rs` (mirrors `share_rooms_ui.rs:130‑139,219‑222`).
6. Remove entirely: `ShareProfiles::remove_room_persisted(default_home, room_profile_id) -> Result<(ShareProfiles, ProfileChange), String>` (`profile_operations.rs:235`) — deletes the profile + secret; follow with the same cleanup + refresh as (5).

### 7. Requests/inbox: list, accept, reject, delete

1. List (read-model, reuse verbatim): `share_lifecycle_view::request_views(profiles: &ShareProfiles, now: i64) -> (Vec<RequestView> /*incoming*/, Vec<RequestView> /*outgoing*/)` and `authorized_device_views(profiles) -> Vec<AuthorizedDeviceView>` (`native/src/app/core/share_lifecycle_view.rs:39,133` — zero egui). Also fold in `profiles.legacy_direct_requests` for old-protocol peers.
2. Queue an outgoing request ("request access" on a saved Direct device): `crate::share::queue_direct_request_for_contact(default_home, identity: &ShareIdentity, contact_id: &str, message: Option<String>) -> Result<DirectRequestAction{entry,created}, String>` (`direct_actions.rs:20`).
3. Accept/Reject/Revoke: `crate::share::decide_direct_request(default_home, identity, request_id: &DirectRequestId, expected_fingerprint: &str, decision: DirectDecisionKind, message: Option<String>) -> Result<DirectRequestEntry, String>` (`direct_actions.rs:100`). Gate Accept client-side the way the GUI does — block when `profiles.tracked_identity_conflict(request_id)` is true (server also enforces this and returns an `Err`, `direct_actions.rs:165‑171`).
4. Retry: `crate::share::retry_direct_request_now(default_home, request_id) -> Result<DirectRequestEntry, String>` (`direct_actions.rs:183`) — only when `entry.manually_retryable_outboxes(now)` is non-empty.
5. Delete: `crate::share::delete_direct_request_history(default_home, request_id) -> Result<(), String>` (`direct_actions.rs:211`) — fails while an accepted incoming request's grant is still active (must revoke first) or while peer delivery is pending.
6. Legacy-protocol equivalents (`decide_legacy_direct_request`, `retry_legacy_direct_answer`, `revoke_legacy_direct_request`, `delete_legacy_direct_request`) — call sites confirmed at `share_legacy_lifecycle_ui.rs:176‑222`; their defining module (`legacy_direct_actions.rs`) was not read in this session.
7. Persist + await: all of the above already write through `ShareProfiles::mutate_persisted`'s compare-and-swap synchronously — no event to await beyond the `Result`. Follow every mutation with `crate::daemon::refresh_share_worker_checked()` and mark "poll now" so the next `drain_share_worker_events()` reflects it immediately (mirrors `refresh_after_action`, `share_lifecycle_ui.rs:459‑470`).

### 8. Exports: list, add, remove local folder exports

1. Read: `profiles.default_direct_exports: ShareExportConfig` (all Direct contacts) or `profiles.rooms[i].exports` (per room) — `{roots: Vec<SharedRoot{label,path}>, include_connections: bool}`.
2. Add: validate the path is an existing directory and canonicalize it (`canonical_directory`, `cli/share/exports.rs:167‑181`, `std::fs::canonicalize` + `is_dir()` check) — the GUI's own text field skips this check, prefer the CLI's validation. Then `ShareProfiles::mutate_persisted(default_home, |profiles| { /* push SharedRoot into the right roots Vec, reject duplicate `path` */ Ok(()) })` (pattern: `exports.rs:74‑91`, `share_exports_ui.rs:17‑33`).
3. Remove: same `mutate_persisted` shape, remove by exact `label` or `path` match, erroring if ambiguous or not found (`exports.rs:93‑123`).
4. After either mutation: `crate::daemon::refresh_share_worker_checked()`.

### 9. One-shot remote command on a peer

1. Request/response types: `ExecRequest{argv: Vec<String>, cwd: Option<String>, timeout_ms: u64, max_output_bytes: u64, shell: bool}` → `ExecResult{stdout: Vec<u8>, stderr: Vec<u8>, exit_code: Option<i32>, timed_out: bool, stdout_truncated: bool, stderr_truncated: bool}` (`share/core/types.rs:19‑35`).
2. Call: `crate::daemon::exec_share(target: PeerOpenTarget, req: ExecRequest) -> Result<ExecResult, String>` (`ipc_client.rs:92`) — a single blocking round-trip; socket timeout = `max(req.timeout_ms + 60_000, 60_000)` ms. This is the simple path; there is no CLI reference for it.
3. Timeout/output limits: enforced by `req.timeout_ms`/`req.max_output_bytes`; surfaced back via `result.timed_out`/`result.stdout_truncated`/`result.stderr_truncated`.
4. Detecting a missing grant: the daemon's `ShareHost::exec_share` (`ipc_host.rs:416‑437`) calls `service.exec_for_target`, defined in `native/src/share/core/service.rs` (**not read in this session**) — the exact error text/shape for "peer has not granted Exec to this device" is unresolved; the facade should treat any `Err(String)` from `exec_share` as a denial-or-failure and surface the message verbatim, and pre-check locally against `ExecGrantTarget`/`ExecGrant.enabled` on the *inbound* side to avoid the round trip when it is already known-disabled (note: this only reflects what the **local** device has granted to *others*, not what a remote peer has granted to *this* device — the remote peer's own decision is authoritative and only visible via the call's `Result`).
5. Managing this device's own outbound Exec grants (what a peer is allowed to run *here*): `crate::daemon::mutate_exec_grant(target: ExecGrantTarget, enabled: bool) -> Result<ExecGrantPersistResult{persisted,applied,revision,retry_state,error}, String>` — IPC `MutateExecGrant`/`ExecGrantMutation` (`ipc_protocol.rs:233,358`); defining client function outside the read surface (see Finding E). Both GUI (`share_exec_ui.rs:210‑237`) and CLI (`cli/share/grants_exec.rs:46‑73`) require an explicit confirmation step before enabling ("FULL … CODE EXECUTION" warning) — replicate that gate in the mobile UI.
6. Job-tracking alternative (richer UX, recommended over the blocking call): `ExecStart{exec_id: ExecId, command: ExecCommand::Argv{program,args}|Shell{command}, cwd, env, timeout_ms, max_output_bytes}` sent as `IpcRequest::ExecStream` (`ipc_protocol.rs:255‑259`), then poll `crate::daemon::exec_jobs() -> Result<ExecJobsSnapshot{incoming_active,outgoing_active,incoming_history,outgoing_history: Vec<ExecJobView>}, String>` and cancel with `crate::daemon::cancel_exec(target: ExecCancelTarget{direction: ExecJobDirection, exec_id, peer_device_id}) -> Result<bool, String>`. **Both functions' defining file (`exec_ipc.rs`/`exec_state.rs`) is outside the four `ipc_*` files assigned to this reading** — only their call sites (`share_exec_jobs_ui.rs:227‑291`, `cli/share/exec_status.rs:36‑117`) and the wire types (`exec_types.rs`) were read. Flagged in Unresolved.

### Ensuring the worker is running / polling status

- Every daemon-facing function in Finding E (`open_share_backend`, `exec_share`, `refresh_share_worker_checked`, `send_share_command`, `drain_share_worker_events`) calls `ensure_worker_ready()` first — it self-heals the background process (spawns/handoff-restarts it if the Ping shows `Missing`/`Stale`, waits out an in-flight `Starting`/`Retiring` generation) and returns once a live daemon answers. The facade does **not** need a separate "start the daemon" step; simply call the action and handle its `Result`.
- `ensure_share()` (`share.rs:50‑131`) additionally: stops any locally-cached `ShareService` handle, requires a non-empty server string, loads/creates the identity, loads profiles, sets the device name, flips `auto_connect=true` if needed (persisted), and finally calls `refresh_share_worker_checked()`.
- Poll loop: call `drain_share_worker_events()` off the UI/main thread on a timer — the desktop GUI uses 300ms while `auto_connect || running`, 900ms otherwise (`share.rs:45‑46`, applied in `share_drain.rs:158‑163`). Apply the returned `snapshot.profiles` as a **full replacement**, never merged, over local state — "the daemon is the sole owner of runtime Share state" (`share_drain.rs:153‑157`). Then fold `snapshot.events` into local UI/notification state; route `ShareEvent::Discovery(_)` through `share_discovery_events::apply_share_discovery_event` (egui-free, reusable as-is, Finding A).

### Committing profile edits instead of a rejected `ShareCmd`

`ShareCmd::EnableExec`, `DisableExec`, `ApplyExecGrant`, and `ConfigureProfiles` are rejected by the daemon over IPC (`ipc_host.rs:315‑324`, exact message `"Dieser Share-Befehl erfordert eine dauerhafte Daemon-Mutation"`). Any other structural change to `ShareProfiles` (adding/removing a contact or room, editing exports, flipping `auto_connect`/`auto_open`/`auto_join`, blocking a room member) must instead:

1. Take a snapshot of the currently-known `ShareProfiles` (`previous`).
2. Edit a local clone (`edited`), or call one of the dedicated `*_persisted` helpers in Finding D directly.
3. Commit via `ShareProfiles::mutate_persisted(default_home, |latest| { profile_edits::merge_user_edits(latest, &previous, &edited); Ok(()) })` when doing a free-form field edit (this is exactly `commit_share_profiles`, `share.rs:189‑205`). `merge_user_edits` (`share_profile_edits.rs:6‑112`, egui-free, reusable verbatim) rebases **only** user-owned fields (`auto_connect`, `default_direct_exports`, per-contact `display_name/auto_connect/auto_open` + a "trust reset" when presence was locally cleared, per-room `name/auto_join/exports`, per-member `blocked`) onto whatever the daemon concurrently wrote, and deliberately leaves daemon-owned runtime fields (`status`, `presence`, `access_state`, `last_seen`, decision state, …) on `latest` untouched. The facade must reuse this merge (or reimplement it identically) to avoid a mobile-side edit clobbering a concurrent daemon-side presence/status update.
4. Call `crate::daemon::refresh_share_worker_checked()` (or send `ShareCmd::Refresh`) so `ShareHost::reload_now_locked` (`ipc_host.rs:208‑302`) re-reads the file and reconciles the live `ShareService` — profile files are **never** hot-applied by an IPC command alone.

Exec-grant enable/disable is the one exception with its own dedicated durable IPC round-trip (`mutate_exec_grant`, item 5 of recipe 9) instead of a profile-file edit.

---

## Decisions

- Treated `native/src/app/core/share.rs` as both the module-declaration hub for the `share_*_ui.rs` family and the primary non-UI connection-lifecycle logic (`ensure_share`/`share_cmd`/`open_share_target`), since that is how the file is actually structured (362 lines, `#[path=...] mod ...;` block followed directly by `impl App`).
- Included `native/src/share/core/identity.rs` and `native/src/share/os/shared/identity_store.rs` even though not explicitly named in the read-surface list, because the assignment's own category ("share/core + share/os/shared files that define the types/commands the GUI uses") and action (1) ("show own device… set device name") make `ShareIdentity` unavoidable; every GUI call site already read (`share.rs`, `share_identity_rotation.rs`, `share_direct_ui.rs`) depends on it directly.
- Did **not** read `native/src/daemon/os/shared/exec_ipc.rs`, `exec_state.rs`, or `native/src/connect/os/shared/resolution.rs`, since they were outside the four named `ipc_*` files and `connect/core/location.rs` respectively; their existence and the functions/types they must define were inferred only from call sites and are flagged below rather than asserted as read facts.
- Reported the CLI-vs-GUI action coverage gaps (Discovery pairing, Room join/list/leave/remove, adding a Direct contact by code, blocking one-shot exec) as findings rather than silently picking whichever side had an implementation, since the assignment explicitly asks which CLI commands "already implement each action... these are the preferred reuse targets" — for several actions the honest answer is "none, only the GUI/core exists."

## Unresolved

- **Configured share server (read path).** No file in the assigned surface reads `share_server.txt` from the client side for display purposes (`App.share_server`'s initial load is outside `native/src/app/core/share*.rs`); only the daemon-side reader (`ipc_host.rs:440‑465`) and the CLI writer (`cli/share.rs:127‑130,210‑224`) were seen.
- **`resolve_endpoint`** (`native/src/connect/os/shared/resolution.rs`) — the function that actually turns a parsed `EndpointSpec::Peer` into an open connection was not read; its behavior for Share targets is inferred, not confirmed, from `connect/core/location.rs` plus the GUI's parallel `open_share_target` pattern.
- **Exec job-tracking client functions** (`crate::daemon::exec_jobs`, `crate::daemon::cancel_exec`, `ExecJobsSnapshot`, `ExecJobDirection`, `ExecCancelTarget`, `mutate_exec_grant`'s own defining file) live in `native/src/daemon/os/shared/exec_ipc.rs` / `exec_state.rs` / `ipc_exec_grant_client.rs`, none of which were in the four assigned `ipc_*` files. Only call sites and wire-level types (`exec_types.rs`, `ipc_protocol.rs`) were confirmed. Recommend a follow-up reading of these three files before finalizing the Exec facade API, since action (9)'s richer job-based flow is very likely the better mobile UX over the blocking `exec_share` call.
- **`crate::share::service::exec_for_target`/`ShareService`** internals (`native/src/share/core/service.rs`, `server.rs`, `server_transfer.rs`) — not read; the exact denial error text/shape when a peer has not granted Exec to the caller is unknown.
- **Legacy direct actions** (`decide_legacy_direct_request`, `retry_legacy_direct_answer`, `revoke_legacy_direct_request`, `delete_legacy_direct_request`, `refresh_legacy_request_expiry`) — only call sites confirmed (`share_legacy_lifecycle_ui.rs`, `share_profile_cache.rs`); their defining file `native/src/share/os/shared/legacy_direct_actions.rs` was not read.
- **"Clear the share server"** has no existing implementation anywhere in the read surface (see recipe 2, step 5) — the mobile facade will need to define this behavior itself if the product wants it.
- **Adding a Direct contact by code** and **Room join/list/leave/remove** have no CLI reference implementation at all (Finding G) — the only egui-free precedent for these specific actions is the already-egui-free portion of the GUI's own action functions (`ShareProfiles::add_direct_from_code_persisted`/`add_room_from_code_persisted`/`remove_room_persisted`, all pure `crate::share` calls with no UI dependency despite living beside UI code), not a separate CLI code path.
- `default_home()`/`default_device_name()` are implemented **three separate times** with slightly different fallbacks: CLI (`cli/share.rs:242‑255`, `USERPROFILE`/`HOME`/temp-dir, `COMPUTERNAME`/`HOSTNAME`/"Smart Explorer CLI"), daemon host (`ipc_host.rs:467‑480`, same env vars, default "Mein Geraet"), and GUI (`dirs_home()`/`default_device_name()` — not defined in any file read in this session, presumably in `native/src/app/core/mod.rs` or a shared app helper). None of the three use a platform-directories crate; all three read `USERPROFILE`/`HOME` env vars directly. The Android facade must supply its own equivalents (app-data dir, device display name) rather than reuse any of these three verbatim — this is exactly the kind of OS fact the `core`/`os` boundary in `AGENTS.md` says must be passed into `core` as a typed value rather than discovered by `core` itself.
