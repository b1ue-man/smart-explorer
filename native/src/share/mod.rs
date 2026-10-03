//! Authenticated Direct/Room Share over QUIC; untrusted rendezvous and relay.

#[path = "core/authorization_policy.rs"]
mod authorization_policy;
#[path = "core/backend.rs"]
mod backend;
#[path = "core/blocking.rs"]
mod blocking;
#[path = "core/configuration_runtime.rs"]
mod configuration_runtime;
#[path = "core/connection_events.rs"]
mod connection_events;
#[path = "core/crypto.rs"]
mod core;
#[path = "os/shared/direct_actions.rs"]
mod direct_actions;
#[cfg(test)]
#[path = "core/direct_identity_conflict_tests.rs"]
mod direct_identity_conflict_tests;
#[path = "core/direct_ledger.rs"]
mod direct_ledger;
#[path = "core/direct_ledger_mutations.rs"]
mod direct_ledger_mutations;
#[path = "core/direct_ledger_projection.rs"]
mod direct_ledger_projection;
#[path = "core/direct_ledger_retention.rs"]
mod direct_ledger_retention;
#[path = "core/direct_ledger_validation.rs"]
mod direct_ledger_validation;
#[path = "core/direct_lifecycle.rs"]
mod direct_lifecycle;
#[path = "core/direct_lifecycle_error.rs"]
mod direct_lifecycle_error;
#[path = "core/direct_messages.rs"]
mod direct_messages;
#[path = "os/shared/direct_policy_store.rs"]
mod direct_policy_store;
#[path = "core/direct_protocol.rs"]
mod direct_protocol;
#[path = "core/direct_reciprocal.rs"]
mod direct_reciprocal;
#[path = "core/direct_reciprocal_coordinator.rs"]
mod direct_reciprocal_coordinator;
#[path = "os/shared/direct_reciprocal_persistence.rs"]
mod direct_reciprocal_persistence;
#[path = "core/direct_reciprocal_session.rs"]
mod direct_reciprocal_session;
#[path = "core/direct_reciprocal_store.rs"]
mod direct_reciprocal_store;
#[path = "core/direct_reciprocal_transport.rs"]
mod direct_reciprocal_transport;
#[path = "core/direct_reciprocal_wire.rs"]
mod direct_reciprocal_wire;
#[path = "core/direct_relation.rs"]
mod direct_relation;
#[path = "os/shared/direct_relation_actions.rs"]
mod direct_relation_actions;
#[path = "os/shared/direct_repair_store_adapter.rs"]
mod direct_repair_store_adapter;
#[path = "core/direct_request_tombstone.rs"]
mod direct_request_tombstone;
#[path = "core/direct_signal_event.rs"]
mod direct_signal_event;
#[path = "core/direct_transcript.rs"]
mod direct_transcript;
#[path = "core/discovery_bundle.rs"]
mod discovery_bundle;
#[path = "core/discovery_domain.rs"]
mod discovery_domain;
#[path = "os/shared/discovery_events.rs"]
pub(crate) mod discovery_events;
#[path = "core/discovery_exchange.rs"]
mod discovery_exchange;
#[path = "core/discovery_exchange_port_impl.rs"]
mod discovery_exchange_port_impl;
#[path = "core/discovery_offer_book.rs"]
mod discovery_offer_book;
#[path = "core/discovery_offer_guard.rs"]
mod discovery_offer_guard;
#[path = "core/discovery_pake.rs"]
mod discovery_pake;
#[path = "core/discovery_pin.rs"]
mod discovery_pin;
#[path = "core/discovery_relation_store.rs"]
mod discovery_relation_store;
#[path = "os/shared/discovery_relation_store_adapter.rs"]
mod discovery_relation_store_adapter;
#[path = "os/shared/discovery_retention.rs"]
pub(crate) mod discovery_retention;
#[path = "core/discovery_signal_cancellation.rs"]
mod discovery_signal_cancellation;
#[path = "core/discovery_signal_commands.rs"]
mod discovery_signal_commands;
#[path = "core/discovery_signal_dispatch.rs"]
mod discovery_signal_dispatch;
#[path = "core/discovery_signal_exchange.rs"]
mod discovery_signal_exchange;
#[path = "core/discovery_signal_maintenance.rs"]
mod discovery_signal_maintenance;
#[path = "core/discovery_signal_offline.rs"]
mod discovery_signal_offline;
#[path = "core/discovery_signal_outcome.rs"]
mod discovery_signal_outcome;
#[path = "core/discovery_signal_persisted.rs"]
mod discovery_signal_persisted;
#[path = "core/discovery_signal_port.rs"]
mod discovery_signal_port;
#[path = "core/discovery_signal_publication.rs"]
mod discovery_signal_publication;
#[path = "core/discovery_signal_state.rs"]
mod discovery_signal_state;
#[path = "core/discovery_signal_types.rs"]
mod discovery_signal_types;
#[path = "core/discovery_signal_validation.rs"]
mod discovery_signal_validation;
#[path = "core/discovery_signal_wire.rs"]
mod discovery_signal_wire;
#[path = "os/shared/discovery_state.rs"]
pub(crate) mod discovery_state;
#[path = "core/discovery_wire.rs"]
mod discovery_wire;
#[path = "core/endpoint_routes.rs"]
mod endpoint_routes;
#[path = "core/exec.rs"]
mod exec;
#[path = "core/exec_auth.rs"]
mod exec_auth;
#[path = "core/exec_client.rs"]
mod exec_client;
#[path = "core/exec_client_active.rs"]
mod exec_client_active;
#[path = "core/exec_frame_reader.rs"]
mod exec_frame_reader;
#[path = "core/exec_grant_runtime.rs"]
mod exec_grant_runtime;
#[path = "core/exec_heartbeat.rs"]
mod exec_heartbeat;
#[path = "core/exec_job.rs"]
mod exec_job;
#[path = "core/exec_platform.rs"]
mod exec_platform;
#[path = "core/exec_policy.rs"]
mod exec_policy;
// Pure `/proc` logic of the Android exec host; the Linux host tests cover it.
#[path = "core/analysis_admission.rs"]
mod analysis_admission;
#[path = "os/shared/analysis_spool.rs"]
mod analysis_spool;
#[path = "os/shared/analysis_tasks.rs"]
mod analysis_tasks;
#[cfg(any(target_os = "android", all(target_os = "linux", test)))]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[path = "os/android/exec_proc.rs"]
mod exec_proc;
#[cfg(all(target_os = "linux", test))]
#[path = "os/android/exec_proc_tests.rs"]
mod exec_proc_tests;
#[path = "core/exec_protocol.rs"]
mod exec_protocol;
#[path = "core/exec_registry.rs"]
mod exec_registry;
#[path = "core/exec_server.rs"]
mod exec_server;
#[path = "core/exec_session.rs"]
mod exec_session;
#[path = "core/exec_supervisor_protocol.rs"]
mod exec_supervisor_protocol;
#[path = "core/exec_targets.rs"]
mod exec_targets;
#[path = "core/exec_types.rs"]
mod exec_types;
#[path = "core/export_config.rs"]
mod export_config;
#[path = "core/framing.rs"]
mod framing;
#[path = "core/fs.rs"]
mod fs;
#[path = "core/fs_access.rs"]
mod fs_access;
#[path = "core/fs_capabilities.rs"]
mod fs_capabilities;
#[path = "core/fs_copy.rs"]
mod fs_copy;
#[path = "core/fs_error.rs"]
mod fs_error;
#[path = "core/fs_paths.rs"]
mod fs_paths;
#[path = "core/fs_response.rs"]
mod fs_response;
#[path = "core/handshake_limits.rs"]
mod handshake_limits;
#[path = "core/host_requests.rs"]
mod host_requests;
#[path = "core/identity.rs"]
mod identity;
#[path = "core/identity_debug.rs"]
mod identity_debug;
#[cfg(not(windows))]
#[path = "os/linux_os/identity_lock.rs"]
mod identity_lock;
#[cfg(windows)]
#[path = "os/windows/identity_lock.rs"]
mod identity_lock;
#[cfg(test)]
#[path = "core/identity_profile_reconciliation_tests.rs"]
mod identity_profile_reconciliation_tests;
#[path = "os/shared/identity_store.rs"]
mod identity_store;
#[path = "core/io_deadline.rs"]
mod io_deadline;
#[path = "core/keepalive.rs"]
mod keepalive;
#[path = "os/shared/lan_link_exchange.rs"]
mod lan_link_exchange;
#[path = "core/lan_link_facts.rs"]
pub(crate) mod lan_link_facts;
#[path = "os/shared/lan_link_transport.rs"]
mod lan_link_transport;
#[path = "core/lan_link_wire.rs"]
mod lan_link_wire;
#[cfg(windows)]
#[path = "os/windows/lan_permission.rs"]
mod lan_permission;
#[cfg(not(windows))]
#[path = "os/shared/lan_permission_unavailable.rs"]
mod lan_permission;
#[path = "os/shared/lan_permission_job.rs"]
mod lan_permission_job;
#[path = "os/shared/lan_presence.rs"]
mod lan_presence;
#[path = "os/shared/lan_presence_auth.rs"]
pub(crate) mod lan_presence_auth;
#[path = "core/lan_presence_match.rs"]
pub(crate) mod lan_presence_match;
#[path = "core/lan_privacy.rs"]
pub(crate) mod lan_privacy;
#[path = "core/lan_settings.rs"]
mod lan_settings;
#[path = "os/shared/lan_settings_store.rs"]
mod lan_settings_store;
#[path = "core/lan_status.rs"]
mod lan_status;
#[path = "os/shared/lan_uplink_evidence.rs"]
pub(crate) mod lan_uplink_evidence;
#[path = "core/lan_uplink_policy.rs"]
pub(crate) mod lan_uplink_policy;
#[path = "os/shared/legacy_direct_actions.rs"]
mod legacy_direct_actions;
#[path = "core/legacy_direct_request.rs"]
mod legacy_direct_request;
#[path = "core/legacy_direct_request_decision.rs"]
mod legacy_direct_request_decision;
#[path = "core/legacy_direct_request_mutations.rs"]
mod legacy_direct_request_mutations;
#[path = "core/legacy_direct_request_reconciliation.rs"]
mod legacy_direct_request_reconciliation;
#[cfg(test)]
#[path = "core/legacy_direct_request_tests.rs"]
mod legacy_direct_request_tests;
#[path = "core/legacy_direct_request_validation.rs"]
mod legacy_direct_request_validation;
#[path = "core/legacy_probe.rs"]
mod legacy_probe;
#[path = "os/shared/legacy_probe_persist.rs"]
mod legacy_probe_persist;
#[path = "os/shared/lifecycle_view.rs"]
pub(crate) mod lifecycle_view;
#[path = "core/line.rs"]
mod line;
#[path = "core/mount_lease.rs"]
mod mount_lease;
#[path = "core/mount_lease_cleanup.rs"]
mod mount_lease_cleanup;
#[path = "core/mount_lease_client.rs"]
mod mount_lease_client;
#[path = "core/node.rs"]
mod node;
#[path = "core/node_accept.rs"]
mod node_accept;
#[path = "core/node_sessions.rs"]
mod node_sessions;
#[path = "core/peer_endpoint_source.rs"]
mod peer_endpoint_source;
#[path = "core/peer_fs_logging.rs"]
mod peer_fs_logging;
#[path = "core/peer_lease_release.rs"]
mod peer_lease_release;
#[path = "core/peer_read.rs"]
mod peer_read;
#[path = "core/peer_request.rs"]
mod peer_request;
#[path = "core/peer_storage_analysis.rs"]
mod peer_storage_analysis;
#[path = "core/peer_storage_snapshot.rs"]
mod peer_storage_snapshot;
#[path = "core/peer_telemetry.rs"]
mod peer_telemetry;
#[path = "core/peer_walk.rs"]
mod peer_walk;
#[path = "core/peer_writer.rs"]
mod peer_writer;
#[cfg(target_os = "android")]
#[path = "os/android/exec.rs"]
mod platform_exec;
#[cfg(target_os = "linux")]
#[path = "os/linux_os/exec.rs"]
mod platform_exec;
#[cfg(windows)]
#[path = "os/windows/exec.rs"]
mod platform_exec;
#[path = "os/shared/poll_status.rs"]
pub(crate) mod poll_status;
#[path = "core/power.rs"]
pub mod power;
#[path = "os/shared/profile_edits.rs"]
pub(crate) mod profile_edits;
#[path = "core/profile_migration.rs"]
mod profile_migration;
#[path = "os/shared/profile_operations.rs"]
mod profile_operations;
#[path = "core/profile_persistence.rs"]
mod profile_persistence;
#[path = "core/profile_policy.rs"]
mod profile_policy;
#[path = "os/shared/profile_policy_actions.rs"]
mod profile_policy_actions;
#[path = "os/shared/profile_store.rs"]
mod profile_store;
#[path = "core/profiles.rs"]
mod profiles;
#[path = "core/relation_rights.rs"]
mod relation_rights;
#[cfg(test)]
#[path = "core/remote_drive_task_mount_lease_tests.rs"]
mod remote_drive_task_mount_lease_tests;
#[path = "os/shared/removal.rs"]
pub(crate) mod removal;
#[path = "core/removed_direct_peers.rs"]
mod removed_direct_peers;
#[path = "core/room_relation.rs"]
mod room_relation;
#[path = "core/server.rs"]
mod server;
#[path = "core/signal_connection_config.rs"]
pub(crate) mod server_address;
#[path = "core/server_capabilities.rs"]
mod server_capabilities;
#[path = "core/server_transfer.rs"]
mod server_transfer;
#[path = "core/service.rs"]
mod service;
#[path = "core/session.rs"]
mod session;
#[cfg(test)]
#[path = "core/share_remote_direct_task_tests.rs"]
mod share_remote_direct_task_tests;
#[cfg(test)]
#[path = "core/share_remote_discovery_task_tests.rs"]
mod share_remote_discovery_task_tests;
#[path = "os/shared/system.rs"]
mod shared_system;
#[path = "core/signal_auth.rs"]
mod signal_auth;
#[path = "core/signal_commands.rs"]
mod signal_commands;
#[cfg(test)]
#[path = "core/signal_configure_tests.rs"]
mod signal_configure_tests;
#[path = "core/signal_connection.rs"]
mod signal_connection;
#[path = "core/signal_connector.rs"]
mod signal_connector;
#[path = "core/signal_handshake.rs"]
mod signal_handshake;
#[path = "core/signal_presence.rs"]
mod signal_presence;
#[path = "core/signal_subscriptions.rs"]
mod signal_subscriptions;
#[path = "core/signal_worker.rs"]
mod signal_worker;
#[path = "os/shared/storage_analysis_host.rs"]
mod storage_analysis_host;
#[path = "core/storage_analysis_server.rs"]
mod storage_analysis_server;
#[path = "core/storage_snapshot.rs"]
mod storage_snapshot;
#[cfg(windows)]
#[path = "os/windows/system.rs"]
mod system;
#[cfg(not(windows))]
#[path = "os/linux_os/system.rs"]
mod system;
#[path = "core/tracked_signal_dispatch.rs"]
mod tracked_signal_dispatch;
#[path = "core/tracked_signal_outbox.rs"]
mod tracked_signal_outbox;
#[path = "core/tracked_signal_sender.rs"]
mod tracked_signal_sender;
#[cfg(test)]
#[path = "core/tracked_signal_sender_tests.rs"]
mod tracked_signal_sender_tests;
#[path = "core/tracked_signal_verify.rs"]
mod tracked_signal_verify;
#[path = "os/shared/transport_options.rs"]
mod transport_options;
#[path = "core/types.rs"]
mod types;
#[path = "core/walk.rs"]
mod walk;
#[path = "core/walk_assembly.rs"]
mod walk_assembly;
#[path = "core/wire.rs"]
mod wire;

mod fs_request {
    pub(in crate::share) use super::wire::FsReversibleReplace;
}

// Preserve facade items and test registrations in the Share namespace.
include!("api_exports.rs");

#[path = "core/analysis_resources.rs"]
mod analysis_resources;
#[path = "core/fair_admission.rs"]
mod fair_admission;
#[path = "core/fs_delete.rs"]
mod fs_delete;
#[path = "core/fs_guard_backend.rs"]
mod fs_guard_backend;
#[path = "os/shared/fs_host_destructive.rs"]
mod fs_host_destructive;
#[path = "os/shared/fs_host_policy.rs"]
mod fs_host_policy;
#[cfg(test)]
#[path = "os/shared/fs_host_policy_task_tests.rs"]
mod fs_host_policy_task_tests;
#[path = "os/shared/fs_local_paths.rs"]
mod fs_local_paths;
#[cfg(windows)]
#[path = "os/windows/fs_path_adapter.rs"]
mod fs_path_adapter;
#[cfg(not(windows))]
#[path = "os/linux_os/fs_path_adapter.rs"]
mod fs_path_adapter;
#[path = "core/fs_policy.rs"]
mod fs_policy;
#[path = "os/shared/host_duplicate_verify.rs"]
mod host_duplicate_verify;
#[path = "os/shared/host_hash_walk.rs"]
mod host_hash_walk;
#[path = "os/shared/host_list.rs"]
mod host_list;
#[path = "os/shared/host_mutations.rs"]
mod host_mutations;
#[path = "os/shared/host_stream.rs"]
mod host_stream;
#[path = "os/shared/host_watch.rs"]
mod host_watch;
#[path = "core/node_policy.rs"]
mod node_policy;
#[path = "core/peer_duplicates.rs"]
mod peer_duplicates;
#[path = "core/peer_extensions.rs"]
mod peer_extensions;
#[path = "core/peer_hash_walk.rs"]
mod peer_hash_walk;
#[path = "core/peer_list_batch.rs"]
mod peer_list_batch;
#[path = "core/peer_stream.rs"]
mod peer_stream;
#[path = "core/peer_watch.rs"]
mod peer_watch;
#[path = "os/shared/storage_duplicate_host.rs"]
mod storage_duplicate_host;
#[path = "os/shared/storage_roots.rs"]
mod storage_roots;
