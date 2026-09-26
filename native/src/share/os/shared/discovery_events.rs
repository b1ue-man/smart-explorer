//! Discovery (PIN pairing) commands and events without UI: a bounded
//! background dispatcher for Share commands, client-side validation of UI
//! actions, and folding daemon events into `DiscoveryUiState`. The caller
//! passes a repaint callback that runs after each finished command.
use super::discovery_state::{
    DiscoveryCompatibility, DiscoveryListEntry, DiscoveryOfferPhase, DiscoveryPublishTarget,
    DiscoveryUiAction, DiscoveryUiKind, DiscoveryUiState, WORKER_STOPPED_STATUS,
};

const DISCOVERY_COMMAND_CAPACITY: usize = 8;

/// Wakes the caller (for example its UI loop) once a background command ran.
pub type DiscoveryRepaint = Box<dyn Fn() + Send>;

pub enum DiscoveryCommandContext {
    Publish(DiscoveryPublishTarget),
    Stop(String),
    Refresh,
    Connect(String),
    Cancel(String),
}

struct DiscoveryCommandRequest {
    context: DiscoveryCommandContext,
    command: crate::share::ShareCmd,
    repaint: DiscoveryRepaint,
}

pub struct DiscoveryCommandResult {
    context: DiscoveryCommandContext,
    result: Result<(), String>,
}

pub struct DiscoveryCommandDispatcher {
    requests: Option<crossbeam_channel::Sender<DiscoveryCommandRequest>>,
    results: crossbeam_channel::Receiver<DiscoveryCommandResult>,
    startup_error: Option<String>,
}

impl DiscoveryCommandDispatcher {
    pub fn new() -> Self {
        let (request_tx, request_rx) =
            crossbeam_channel::bounded::<DiscoveryCommandRequest>(DISCOVERY_COMMAND_CAPACITY);
        let (result_tx, result_rx) = crossbeam_channel::unbounded();
        let spawned = std::thread::Builder::new()
            .name("share-discovery-command".into())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let result = crate::daemon::send_share_command(request.command);
                    if result_tx
                        .send(DiscoveryCommandResult {
                            context: request.context,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                    (request.repaint)();
                }
            });
        match spawned {
            Ok(_) => Self {
                requests: Some(request_tx),
                results: result_rx,
                startup_error: None,
            },
            Err(error) => Self {
                requests: None,
                results: result_rx,
                startup_error: Some(format!(
                    "Discovery-Hintergrundbefehle konnten nicht gestartet werden: {error}"
                )),
            },
        }
    }

    pub fn startup_error(&self) -> Option<&str> {
        self.startup_error.as_deref()
    }

    pub fn submit(
        &self,
        context: DiscoveryCommandContext,
        command: crate::share::ShareCmd,
        repaint: DiscoveryRepaint,
    ) -> Result<(), (DiscoveryCommandContext, String)> {
        let Some(requests) = &self.requests else {
            return Err((
                context,
                self.startup_error
                    .clone()
                    .unwrap_or_else(|| "Discovery-Befehlsworker ist nicht verfuegbar".into()),
            ));
        };
        let request = DiscoveryCommandRequest {
            context,
            command,
            repaint,
        };
        match requests.try_send(request) {
            Ok(()) => Ok(()),
            Err(crossbeam_channel::TrySendError::Full(request)) => Err((
                request.context,
                "Discovery-Befehlswarteschlange ist ausgelastet".into(),
            )),
            Err(crossbeam_channel::TrySendError::Disconnected(request)) => Err((
                request.context,
                "Discovery-Befehlsworker wurde beendet".into(),
            )),
        }
    }

    pub fn drain(&self) -> Vec<DiscoveryCommandResult> {
        self.results.try_iter().collect()
    }
}

/// Rolls back the pending state of every finished command that failed.
pub fn drain_discovery_command_results(state: &mut DiscoveryUiState) {
    let results = state.dispatcher.drain();
    for result in results {
        if let Err(error) = result.result {
            apply_discovery_command_failure(state, result.context, error);
        }
    }
}

/// Validates `action`, marks it pending in `state` and queues its command.
pub fn dispatch_discovery_ui_action(
    state: &mut DiscoveryUiState,
    action: DiscoveryUiAction,
    repaint: DiscoveryRepaint,
) {
    let prepared = match action {
        DiscoveryUiAction::Publish {
            target,
            display_alias,
            pin,
            duration_secs,
        } => {
            if duration_secs == 0 {
                state.command_error("Die Sichtbarkeitsdauer muss positiv sein".into());
                return;
            }
            if pin.as_bytes().len() > crate::share::DISCOVERY_PIN_MAX_BYTES {
                state.command_error(pin_limit_error(pin.as_bytes().len()));
                return;
            }
            if !state.begin_publish(&target) {
                return;
            }
            let command_target = match &target {
                DiscoveryPublishTarget::Direct => crate::share::DiscoveryPublishTarget::Direct,
                DiscoveryPublishTarget::Room { room_id, .. } => {
                    crate::share::DiscoveryPublishTarget::Room {
                        room_profile_id: room_id.clone(),
                    }
                }
            };
            (
                DiscoveryCommandContext::Publish(target),
                crate::share::ShareCmd::Discovery(crate::share::DiscoveryCommand::Publish {
                    target: command_target,
                    display_alias,
                    pin,
                    duration_secs,
                }),
            )
        }
        DiscoveryUiAction::Stop { offer_id } => {
            if !state.stop_started(&offer_id) {
                return;
            }
            (
                DiscoveryCommandContext::Stop(offer_id.clone()),
                crate::share::ShareCmd::Discovery(crate::share::DiscoveryCommand::StopPublishing {
                    offer_id,
                }),
            )
        }
        DiscoveryUiAction::Refresh => {
            if state.refreshing {
                return;
            }
            state.refreshing = true;
            (
                DiscoveryCommandContext::Refresh,
                crate::share::ShareCmd::Discovery(crate::share::DiscoveryCommand::ListDiscoveries),
            )
        }
        DiscoveryUiAction::Connect { discovery_id, pin } => {
            if pin.as_bytes().len() > crate::share::DISCOVERY_PIN_MAX_BYTES {
                state.command_error(pin_limit_error(pin.as_bytes().len()));
                return;
            }
            if !state.connect_started(&discovery_id) {
                return;
            }
            state.entry_pins.remove(&discovery_id);
            (
                DiscoveryCommandContext::Connect(discovery_id.clone()),
                crate::share::ShareCmd::Discovery(
                    crate::share::DiscoveryCommand::StartDiscoveryExchange { discovery_id, pin },
                ),
            )
        }
        DiscoveryUiAction::Cancel { exchange_id } => {
            if !state.cancel_started(&exchange_id) {
                return;
            }
            (
                DiscoveryCommandContext::Cancel(exchange_id.clone()),
                crate::share::ShareCmd::Discovery(
                    crate::share::DiscoveryCommand::CancelDiscoveryExchange { exchange_id },
                ),
            )
        }
    };
    if let Err((context, error)) = state.dispatcher.submit(prepared.0, prepared.1, repaint) {
        apply_discovery_command_failure(state, context, error);
    }
}

fn apply_discovery_command_failure(
    state: &mut DiscoveryUiState,
    context: DiscoveryCommandContext,
    error: String,
) {
    match context {
        DiscoveryCommandContext::Publish(target) => state.publish_command_failed(&target),
        DiscoveryCommandContext::Stop(offer_id) => state.stop_command_failed(&offer_id),
        DiscoveryCommandContext::Refresh => state.refreshing = false,
        DiscoveryCommandContext::Connect(discovery_id) => {
            state.connect_command_failed(&discovery_id)
        }
        DiscoveryCommandContext::Cancel(exchange_id) => state.cancel_command_failed(&exchange_id),
    }
    state.command_error(error);
}

/// Folds one daemon discovery event into `state`.
pub fn apply_share_discovery_event(
    state: &mut DiscoveryUiState,
    event: crate::share::DiscoveryEvent,
) {
    match event {
        crate::share::DiscoveryEvent::OfferPrepared {
            offer_id,
            target,
            display_alias,
            discoverable_until,
        } => state.offer_updated(
            offer_id,
            map_publish_target(target, display_alias),
            discoverable_until,
            DiscoveryOfferPhase::Prepared,
        ),
        crate::share::DiscoveryEvent::OfferPublished {
            offer_id,
            target,
            display_alias,
            discoverable_until,
        } => state.offer_updated(
            offer_id,
            map_publish_target(target, display_alias),
            discoverable_until,
            DiscoveryOfferPhase::Published,
        ),
        crate::share::DiscoveryEvent::OfferStopped { offer_id, reason } => {
            let status = match reason {
                crate::share::DiscoveryOfferStopReason::Requested => "Sichtbarkeit beendet",
                crate::share::DiscoveryOfferStopReason::Expired => {
                    "Sichtbarkeit planmaessig abgelaufen"
                }
                crate::share::DiscoveryOfferStopReason::CapabilityUnavailable => {
                    "Sichtbarkeit beendet: Server unterstuetzt Discovery nicht"
                }
                crate::share::DiscoveryOfferStopReason::TransportError => {
                    "Sichtbarkeit wegen eines Verbindungsfehlers beendet"
                }
                crate::share::DiscoveryOfferStopReason::TargetUnavailable => {
                    "Sichtbarkeit beendet: Ziel ist nicht mehr verfuegbar"
                }
                crate::share::DiscoveryOfferStopReason::WorkerStopped => WORKER_STOPPED_STATUS,
            };
            state.stopped(&offer_id);
            state.status = Some(status.to_string());
        }
        crate::share::DiscoveryEvent::DiscoveryList { advertisements } => {
            let mut entries: Vec<_> = advertisements.into_iter().map(list_entry).collect();
            entries.sort_by(|left, right| {
                left.display_alias
                    .to_lowercase()
                    .cmp(&right.display_alias.to_lowercase())
                    .then_with(|| left.discovery_id.cmp(&right.discovery_id))
            });
            state.replace_list(entries);
        }
        crate::share::DiscoveryEvent::ExchangeStarted {
            exchange_id,
            discovery_id,
        } => state.exchange_started(exchange_id, discovery_id),
        crate::share::DiscoveryEvent::ExchangeCompleted {
            exchange_id,
            discovery_id,
            outcome,
        } => state.exchange_completed(exchange_id, discovery_id, outcome_label(outcome)),
        crate::share::DiscoveryEvent::ExchangeCancelled {
            exchange_id,
            discovery_id,
        } => state.exchange_cancelled(exchange_id, discovery_id),
        crate::share::DiscoveryEvent::ExchangeFailed {
            exchange_id,
            discovery_id,
            error,
        } => state.exchange_failed(exchange_id, discovery_id, error),
    }
}

fn pin_limit_error(bytes: usize) -> String {
    format!(
        "PIN ist {bytes} Bytes lang; maximal {} Bytes sind erlaubt",
        crate::share::DISCOVERY_PIN_MAX_BYTES
    )
}

fn outcome_label(outcome: crate::share::DiscoveryRelationOutcome) -> String {
    match outcome {
        crate::share::DiscoveryRelationOutcome::DirectInstalled { display_name, .. } => {
            format!("Direktkontakt {display_name}")
        }
        crate::share::DiscoveryRelationOutcome::RoomInstalled { display_name, .. } => {
            format!("Raum {display_name} hinzugefuegt")
        }
        crate::share::DiscoveryRelationOutcome::RoomShared { display_name, .. } => {
            format!("Raum {display_name} geteilt")
        }
    }
}

fn list_entry(advertisement: crate::share::DiscoveryAdvertisement) -> DiscoveryListEntry {
    let compatibility = if advertisement.is_compatible() {
        DiscoveryCompatibility::Compatible
    } else if advertisement.suite != crate::share::DISCOVERY_PAIRING_SUITE {
        DiscoveryCompatibility::UnsupportedSuite
    } else {
        DiscoveryCompatibility::UnsupportedVersion
    };
    DiscoveryListEntry {
        discovery_id: advertisement.discovery_id,
        kind: map_kind(advertisement.kind),
        display_alias: advertisement.display_alias,
        expires_at: advertisement.expires_at,
        compatibility,
    }
}

fn map_kind(kind: crate::share::DiscoveryKind) -> DiscoveryUiKind {
    match kind {
        crate::share::DiscoveryKind::Direct => DiscoveryUiKind::Direct,
        crate::share::DiscoveryKind::Room => DiscoveryUiKind::Room,
    }
}

fn map_publish_target(
    target: crate::share::DiscoveryPublishTarget,
    display_alias: String,
) -> DiscoveryPublishTarget {
    match target {
        crate::share::DiscoveryPublishTarget::Direct => DiscoveryPublishTarget::Direct,
        crate::share::DiscoveryPublishTarget::Room { room_profile_id } => {
            DiscoveryPublishTarget::Room {
                room_id: room_profile_id,
                room_name: display_alias,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_share_discovery_event, DiscoveryUiState, WORKER_STOPPED_STATUS};
    use crate::share::{DiscoveryEvent, DiscoveryOfferStopReason, OwnDiscoveryOffer};

    fn published(offer_id: &str) -> DiscoveryEvent {
        DiscoveryEvent::OfferPublished {
            offer_id: offer_id.into(),
            target: crate::share::DiscoveryPublishTarget::Room {
                room_profile_id: format!("room-{offer_id}"),
            },
            display_alias: "Team".into(),
            discoverable_until: i64::MAX,
        }
    }

    #[test]
    fn cli_task_gui_state_drops_stopped_and_vanished_offers() {
        let mut state = DiscoveryUiState::default();
        apply_share_discovery_event(&mut state, published("stopped"));
        apply_share_discovery_event(&mut state, published("vanished"));
        apply_share_discovery_event(&mut state, published("kept"));
        assert_eq!(state.active_offers.len(), 3);

        let stop = DiscoveryEvent::OfferStopped {
            offer_id: "stopped".into(),
            reason: DiscoveryOfferStopReason::WorkerStopped,
        };
        apply_share_discovery_event(&mut state, stop);
        assert_eq!(state.active_offers.len(), 2);
        assert_eq!(state.status.take().as_deref(), Some(WORKER_STOPPED_STATUS));

        // After a handoff the new worker lists only what it runs.
        let kept = OwnDiscoveryOffer {
            offer_id: "kept".into(),
            target: crate::share::DiscoveryPublishTarget::Room {
                room_profile_id: "room-kept".into(),
            },
            display_alias: "Team".into(),
            discoverable_until: i64::MAX,
            published: true,
        };
        state.retain_live_offers(std::slice::from_ref(&kept));
        let ids: Vec<_> = state
            .active_offers
            .iter()
            .map(|offer| offer.offer_id.as_str())
            .collect();
        assert_eq!(ids, ["kept"]);
        assert_eq!(state.status.take().as_deref(), Some(WORKER_STOPPED_STATUS));
        state.retain_live_offers(std::slice::from_ref(&kept));
        assert!(state.status.is_none());
    }
}
