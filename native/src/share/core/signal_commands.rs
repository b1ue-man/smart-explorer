use std::collections::HashSet;
use std::io;
use std::sync::{Arc, Mutex};

use super::authorization_policy::configuration_changed;
use super::backend::ShareIrohNode;
use super::core::eio;
use super::signal_connection::{send_line, SignalConnection};
use super::signal_subscriptions::{plan_subscription_teardown, SubscriptionTeardownPlan};
use super::signal_worker::{publish_all, send_direct_answer, send_direct_request};
use super::tracked_signal_sender::{send_pending_tracked, AttemptCounters};
use super::types::{ShareAuthState, ShareCmd, ShareCmdResult, ShareEvent};
use super::wire::ClientMsg;

pub(super) struct ConnectedCommandRuntime<'a> {
    pub(super) signal: &'a mut SignalConnection,
    pub(super) auth: &'a Arc<Mutex<ShareAuthState>>,
    pub(super) iroh: &'a ShareIrohNode,
    pub(super) direct_requests_sent: &'a mut HashSet<String>,
    pub(super) tracked_direct: bool,
    pub(super) events: &'a crossbeam_channel::Sender<ShareEvent>,
    pub(super) tracked_attempts: &'a mut AttemptCounters,
}

pub(super) struct OfflineCommandRuntime<'a> {
    pub(super) auth: &'a Arc<Mutex<ShareAuthState>>,
    pub(super) iroh: &'a ShareIrohNode,
    pub(super) direct_requests_sent: &'a mut HashSet<String>,
    pub(super) events: &'a crossbeam_channel::Sender<ShareEvent>,
}

pub(super) struct CommandOutcome {
    pub(super) result: io::Result<ShareCmdResult>,
    pub(super) should_stop: bool,
    pub(super) should_reconnect: bool,
    pub(super) published: bool,
}

impl CommandOutcome {
    fn local(result: io::Result<()>) -> Self {
        Self {
            result: result.map(|()| ShareCmdResult::Applied),
            should_stop: false,
            should_reconnect: false,
            published: false,
        }
    }

    fn connected(result: io::Result<()>, published: bool) -> Self {
        let published = published && result.is_ok();
        Self {
            result: result.map(|()| ShareCmdResult::Applied),
            should_stop: false,
            should_reconnect: false,
            published,
        }
    }

    fn connected_fail_closed(result: io::Result<()>, published: bool) -> Self {
        let should_reconnect = result.is_err();
        let published = published && result.is_ok();
        Self {
            result: result.map(|()| ShareCmdResult::Applied),
            should_stop: false,
            should_reconnect,
            published,
        }
    }

    fn stop() -> Self {
        Self {
            result: Ok(ShareCmdResult::Applied),
            should_stop: true,
            should_reconnect: false,
            published: false,
        }
    }

    fn exec(result: io::Result<super::exec_grant_runtime::ExecGrantMutation>) -> Self {
        Self {
            result: result.map(|mutation| ShareCmdResult::ExecGrant(Box::new(mutation))),
            should_stop: false,
            should_reconnect: false,
            published: false,
        }
    }
}

pub(super) fn run_connected_command(
    command: ShareCmd,
    runtime: &mut ConnectedCommandRuntime<'_>,
) -> CommandOutcome {
    match command {
        ShareCmd::Configure {
            direct,
            direct_grants,
            rooms,
            default_direct_exports,
        } => {
            let result = plan_current_subscription_teardown(runtime.auth, &direct, &rooms)
                .and_then(|teardown| {
                    apply_configuration(
                        runtime.auth,
                        runtime.iroh,
                        runtime.direct_requests_sent,
                        direct,
                        direct_grants,
                        rooms,
                        default_direct_exports,
                    )?;
                    send_subscription_teardown(runtime.signal, teardown)
                })
                .and_then(|()| {
                    publish_all(
                        runtime.signal,
                        runtime.auth,
                        runtime.iroh,
                        runtime.direct_requests_sent,
                        runtime.tracked_direct,
                    )
                });
            // Configuration may already be committed locally when session
            // invalidation, teardown, or republishing fails. Drop this
            // registration so server cleanup removes every old subscription;
            // the reconnect publishes only the canonical new local state.
            CommandOutcome::connected_fail_closed(result, true)
        }
        ShareCmd::SyncDirectRequests {
            direct_requests,
            direct_request_tombstones,
        } => {
            let result =
                sync_direct_requests(runtime.auth, direct_requests, direct_request_tombstones)
                    .and_then(|()| {
                        if runtime.tracked_direct {
                            send_pending_tracked(
                                runtime.signal,
                                runtime.auth,
                                runtime.iroh,
                                runtime.events,
                                runtime.tracked_attempts,
                            )
                            .map(|_| ())
                        } else {
                            Ok(())
                        }
                    });
            CommandOutcome::connected(result, false)
        }
        ShareCmd::Refresh => CommandOutcome::connected(
            publish_all(
                runtime.signal,
                runtime.auth,
                runtime.iroh,
                runtime.direct_requests_sent,
                runtime.tracked_direct,
            ),
            true,
        ),
        ShareCmd::SetDirectOnline { online } => {
            let result =
                set_direct_online(runtime.auth, runtime.iroh, online).and_then(|lookup_id| {
                    if online {
                        publish_all(
                            runtime.signal,
                            runtime.auth,
                            runtime.iroh,
                            runtime.direct_requests_sent,
                            runtime.tracked_direct,
                        )
                    } else {
                        send_line(runtime.signal, &ClientMsg::UnpublishDirect { lookup_id })
                    }
                });
            CommandOutcome::connected(result, online)
        }
        ShareCmd::EnableExec { target } => {
            CommandOutcome::exec(mutate_exec_grant(runtime.auth, runtime.iroh, target, true))
        }
        ShareCmd::DisableExec { target } => {
            CommandOutcome::exec(mutate_exec_grant(runtime.auth, runtime.iroh, target, false))
        }
        ShareCmd::ApplyExecGrant {
            target,
            principal,
            policy,
        } => CommandOutcome::exec(apply_persisted_exec_grant(
            runtime.auth,
            runtime.iroh,
            target,
            *principal,
            policy,
        )),
        ShareCmd::Stop => CommandOutcome::stop(),
        ShareCmd::LeaveRoom { room_id } => CommandOutcome::connected(
            send_line(runtime.signal, &ClientMsg::LeaveRoom { room_id }),
            false,
        ),
        ShareCmd::RequestDirect { contact_id } => {
            let result = if runtime.tracked_direct {
                send_pending_tracked(
                    runtime.signal,
                    runtime.auth,
                    runtime.iroh,
                    runtime.events,
                    runtime.tracked_attempts,
                )
                .map(|_| ())
            } else {
                send_direct_request(runtime.signal, runtime.auth, runtime.iroh, &contact_id)
            };
            if result.is_ok() && !runtime.tracked_direct {
                runtime.direct_requests_sent.insert(contact_id);
            }
            CommandOutcome::connected(result, false)
        }
        ShareCmd::AnswerLegacyDirectRequest {
            selector: _,
            decision_revision: _,
            lookup_id,
            requester_device_id,
            accepted,
        } => {
            // This command exists only for a verified legacy request. It must
            // keep using the legacy wire even when the server also negotiated
            // tracked_direct_v1; an old requester cannot consume a signed
            // decision envelope.
            let result = send_direct_answer(
                runtime.signal,
                runtime.auth,
                runtime.iroh,
                lookup_id,
                requester_device_id,
                accepted,
            );
            CommandOutcome::connected(result, false)
        }
    }
}

pub(super) fn run_offline_command(
    command: ShareCmd,
    runtime: &mut OfflineCommandRuntime<'_>,
) -> CommandOutcome {
    match command {
        ShareCmd::Configure {
            direct,
            direct_grants,
            rooms,
            default_direct_exports,
        } => CommandOutcome::local(apply_configuration(
            runtime.auth,
            runtime.iroh,
            runtime.direct_requests_sent,
            direct,
            direct_grants,
            rooms,
            default_direct_exports,
        )),
        ShareCmd::SyncDirectRequests {
            direct_requests,
            direct_request_tombstones,
        } => CommandOutcome::local(sync_direct_requests(
            runtime.auth,
            direct_requests,
            direct_request_tombstones,
        )),
        ShareCmd::Refresh => {
            let _ = runtime.events.send(ShareEvent::Status(
                "Share-Aktualisierung lokal vorgemerkt; Signaling nicht verbunden".into(),
            ));
            CommandOutcome::local(Ok(()))
        }
        ShareCmd::SetDirectOnline { online } => {
            CommandOutcome::local(set_direct_online(runtime.auth, runtime.iroh, online).map(|_| ()))
        }
        ShareCmd::EnableExec { target } => {
            CommandOutcome::exec(mutate_exec_grant(runtime.auth, runtime.iroh, target, true))
        }
        ShareCmd::DisableExec { target } => {
            CommandOutcome::exec(mutate_exec_grant(runtime.auth, runtime.iroh, target, false))
        }
        ShareCmd::ApplyExecGrant {
            target,
            principal,
            policy,
        } => CommandOutcome::exec(apply_persisted_exec_grant(
            runtime.auth,
            runtime.iroh,
            target,
            *principal,
            policy,
        )),
        ShareCmd::Stop => CommandOutcome::stop(),
        ShareCmd::LeaveRoom { .. }
        | ShareCmd::RequestDirect { .. }
        | ShareCmd::AnswerLegacyDirectRequest { .. } => CommandOutcome::local(Err(eio(
            "Share-Server nicht verbunden; Netzwerkkommando wurde nicht gesendet",
        ))),
    }
}

fn plan_current_subscription_teardown(
    auth: &Arc<Mutex<ShareAuthState>>,
    direct: &[super::types::DirectContact],
    rooms: &[super::types::RoomProfile],
) -> io::Result<SubscriptionTeardownPlan> {
    let state = auth.lock().map_err(|_| eio("Share-State gesperrt"))?;
    Ok(plan_subscription_teardown(&state, direct, rooms))
}

fn send_subscription_teardown(
    signal: &mut SignalConnection,
    teardown: SubscriptionTeardownPlan,
) -> io::Result<()> {
    for lookup_id in teardown.direct_lookup_ids {
        send_line(signal, &ClientMsg::UnwatchDirect { lookup_id })?;
    }
    for room_id in teardown.room_ids {
        send_line(signal, &ClientMsg::LeaveRoom { room_id })?;
    }
    Ok(())
}

fn mutate_exec_grant(
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    target: super::types::ExecGrantTarget,
    enabled: bool,
) -> io::Result<super::exec_grant_runtime::ExecGrantMutation> {
    let mutation = super::exec_grant_runtime::mutate(
        auth,
        iroh.exec_registry(),
        target,
        enabled,
        super::core::now_secs(),
    )?;
    // Exec authorization is checked on every fresh Exec connection, and the
    // registry has already installed the deny barrier/cancellation above.
    // Keep existing connections alive long enough to deliver their signed-off
    // Revoked terminal status; closing QUIC here would erase that lifecycle
    // distinction and surface only an ambiguous disconnect to the requester.
    Ok(mutation)
}

fn apply_persisted_exec_grant(
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    target: super::types::ExecGrantTarget,
    principal: super::exec_types::ExecPrincipal,
    policy: super::exec_policy::ExecGrant,
) -> io::Result<super::exec_grant_runtime::ExecGrantMutation> {
    let mutation = super::exec_grant_runtime::apply_exact(
        auth,
        iroh.exec_registry(),
        target,
        principal,
        policy,
    )?;
    Ok(mutation)
}

fn apply_configuration(
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    direct_requests_sent: &mut HashSet<String>,
    direct: Vec<super::types::DirectContact>,
    direct_grants: Vec<super::types::DirectGrant>,
    rooms: Vec<super::types::RoomProfile>,
    default_direct_exports: super::fs::ShareExportConfig,
) -> io::Result<()> {
    let changed = {
        let mut state = auth.lock().map_err(|_| eio("Share-State gesperrt"))?;
        let changed = configuration_changed(
            &state,
            &direct,
            &direct_grants,
            &rooms,
            &default_direct_exports,
        );
        if changed {
            let next_epoch = state
                .authorization_epoch
                .checked_add(1)
                .ok_or_else(|| eio("Share authorization epoch exhausted"))?;
            let mut candidate = state.clone();
            candidate.direct_contacts = direct;
            candidate.direct_grants = direct_grants;
            candidate.rooms = rooms;
            candidate.default_direct_exports = default_direct_exports;
            candidate.authorization_epoch = next_epoch;
            super::exec_grant_runtime::apply_configuration_transition(
                &state,
                &candidate,
                next_epoch,
                iroh.exec_registry(),
            )?;
            *state = candidate;
        } else {
            state.direct_contacts = direct;
            state.direct_grants = direct_grants;
            state.rooms = rooms;
            state.default_direct_exports = default_direct_exports;
        }
        changed
    };
    if changed {
        iroh.invalidate_sessions()?;
        direct_requests_sent.clear();
    }
    Ok(())
}

fn sync_direct_requests(
    auth: &Arc<Mutex<ShareAuthState>>,
    direct_requests: Vec<super::direct_ledger::DirectRequestEntry>,
    direct_request_tombstones: Vec<super::direct_request_tombstone::DirectRequestTombstone>,
) -> io::Result<()> {
    auth.lock()
        .map_err(|_| eio("Share-State gesperrt"))
        .map(|mut state| {
            state.direct_requests = direct_requests;
            state.direct_request_tombstones = direct_request_tombstones;
        })
}

fn set_direct_online(
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    online: bool,
) -> io::Result<String> {
    let (lookup_id, changed) = {
        let mut state = auth.lock().map_err(|_| eio("Share-State gesperrt"))?;
        let changed = state.direct_online != online;
        if changed {
            let next_epoch = state
                .authorization_epoch
                .checked_add(1)
                .ok_or_else(|| eio("Share authorization epoch exhausted"))?;
            let mut candidate = state.clone();
            candidate.direct_online = online;
            candidate.authorization_epoch = next_epoch;
            super::exec_grant_runtime::apply_configuration_transition(
                &state,
                &candidate,
                next_epoch,
                iroh.exec_registry(),
            )?;
            *state = candidate;
        }
        (state.identity.direct_lookup_id.clone(), changed)
    };
    if changed {
        iroh.invalidate_sessions()?;
    }
    Ok(lookup_id)
}
