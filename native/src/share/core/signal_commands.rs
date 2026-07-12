use std::collections::HashSet;
use std::io;
use std::sync::{Arc, Mutex};

use super::authorization_policy::configuration_changed;
use super::backend::ShareIrohNode;
use super::core::eio;
use super::signal_connection::{send_line, SignalConnection};
use super::signal_worker::{publish_all, send_direct_answer, send_direct_request};
use super::tracked_signal_sender::{send_pending_tracked, AttemptCounters};
use super::types::{ShareAuthState, ShareCmd, ShareEvent};
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
    pub(super) result: io::Result<()>,
    pub(super) should_stop: bool,
    pub(super) published: bool,
}

impl CommandOutcome {
    fn local(result: io::Result<()>) -> Self {
        Self {
            result,
            should_stop: false,
            published: false,
        }
    }

    fn connected(result: io::Result<()>, published: bool) -> Self {
        Self {
            result,
            should_stop: false,
            published,
        }
    }

    fn stop() -> Self {
        Self {
            result: Ok(()),
            should_stop: true,
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
            let result = apply_configuration(
                runtime.auth,
                runtime.iroh,
                runtime.direct_requests_sent,
                direct,
                direct_grants,
                rooms,
                default_direct_exports,
            )
            .and_then(|()| {
                publish_all(
                    runtime.signal,
                    runtime.auth,
                    runtime.iroh,
                    runtime.direct_requests_sent,
                    runtime.tracked_direct,
                )
            });
            CommandOutcome::connected(result, true)
        }
        ShareCmd::SyncDirectRequests { direct_requests } => {
            let result = sync_direct_requests(runtime.auth, direct_requests).and_then(|()| {
                if runtime.tracked_direct {
                    send_pending_tracked(
                        runtime.signal,
                        runtime.auth,
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
        ShareCmd::AnswerDirectRequest {
            lookup_id,
            presence,
            accepted,
        } => {
            let result = if runtime.tracked_direct {
                send_pending_tracked(
                    runtime.signal,
                    runtime.auth,
                    runtime.events,
                    runtime.tracked_attempts,
                )
                .map(|_| ())
            } else {
                send_direct_answer(
                    runtime.signal,
                    runtime.auth,
                    runtime.iroh,
                    lookup_id,
                    presence,
                    accepted,
                )
            };
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
        ShareCmd::SyncDirectRequests { direct_requests } => {
            CommandOutcome::local(sync_direct_requests(runtime.auth, direct_requests))
        }
        ShareCmd::Refresh => {
            let _ = runtime.events.send(ShareEvent::Status(
                "Share-Aktualisierung lokal vorgemerkt; Signaling nicht verbunden".into(),
            ));
            CommandOutcome::local(Ok(()))
        }
        ShareCmd::SetDirectOnline { online } => {
            CommandOutcome::local(set_direct_online(runtime.auth, runtime.iroh, online).map(|_| ()))
        }
        ShareCmd::Stop => CommandOutcome::stop(),
        ShareCmd::LeaveRoom { .. }
        | ShareCmd::RequestDirect { .. }
        | ShareCmd::AnswerDirectRequest { .. } => CommandOutcome::local(Err(eio(
            "Share-Server nicht verbunden; Netzwerkkommando wurde nicht gesendet",
        ))),
    }
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
    let changed = auth
        .lock()
        .map_err(|_| eio("Share-State gesperrt"))
        .map(|mut state| {
            let changed = configuration_changed(
                &state,
                &direct,
                &direct_grants,
                &rooms,
                &default_direct_exports,
            );
            state.direct_contacts = direct;
            state.direct_grants = direct_grants;
            state.rooms = rooms;
            state.default_direct_exports = default_direct_exports;
            changed
        })?;
    if changed {
        iroh.invalidate_sessions()?;
        direct_requests_sent.clear();
    }
    Ok(())
}

fn sync_direct_requests(
    auth: &Arc<Mutex<ShareAuthState>>,
    direct_requests: Vec<super::direct_ledger::DirectRequestEntry>,
) -> io::Result<()> {
    auth.lock()
        .map_err(|_| eio("Share-State gesperrt"))
        .map(|mut state| state.direct_requests = direct_requests)
}

fn set_direct_online(
    auth: &Arc<Mutex<ShareAuthState>>,
    iroh: &ShareIrohNode,
    online: bool,
) -> io::Result<String> {
    let (lookup_id, changed) =
        auth.lock()
            .map_err(|_| eio("Share-State gesperrt"))
            .map(|mut state| {
                let changed = state.direct_online != online;
                state.direct_online = online;
                (state.identity.direct_lookup_id.clone(), changed)
            })?;
    if changed {
        iroh.invalidate_sessions()?;
    }
    Ok(lookup_id)
}
