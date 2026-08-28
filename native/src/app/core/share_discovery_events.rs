use super::share_discovery_state::{
    DiscoveryCompatibility, DiscoveryListEntry, DiscoveryOfferPhase, DiscoveryPublishTarget,
    DiscoveryUiAction, DiscoveryUiKind,
};
use super::App;
use eframe::egui;

const DISCOVERY_COMMAND_CAPACITY: usize = 8;

pub(in crate::app) enum DiscoveryCommandContext {
    Publish(DiscoveryPublishTarget),
    Stop(String),
    Refresh,
    Connect(String),
    Cancel(String),
}

struct DiscoveryCommandRequest {
    context: DiscoveryCommandContext,
    command: crate::share::ShareCmd,
    repaint: egui::Context,
}

pub(in crate::app) struct DiscoveryCommandResult {
    context: DiscoveryCommandContext,
    result: Result<(), String>,
}

pub(in crate::app) struct DiscoveryCommandDispatcher {
    requests: Option<crossbeam_channel::Sender<DiscoveryCommandRequest>>,
    results: crossbeam_channel::Receiver<DiscoveryCommandResult>,
    startup_error: Option<String>,
}

impl DiscoveryCommandDispatcher {
    pub(in crate::app) fn new() -> Self {
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
                    request.repaint.request_repaint();
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

    pub(in crate::app) fn startup_error(&self) -> Option<&str> {
        self.startup_error.as_deref()
    }

    pub(in crate::app) fn submit(
        &self,
        context: DiscoveryCommandContext,
        command: crate::share::ShareCmd,
        repaint: egui::Context,
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

    pub(in crate::app) fn drain(&self) -> Vec<DiscoveryCommandResult> {
        self.results.try_iter().collect()
    }
}

impl App {
    pub(in crate::app) fn drain_discovery_command_results(&mut self) {
        let results = self.share_discovery.dispatcher.drain();
        for result in results {
            if let Err(error) = result.result {
                self.apply_discovery_command_failure(result.context, error);
            }
        }
    }

    pub(in crate::app) fn dispatch_discovery_ui_action(
        &mut self,
        action: DiscoveryUiAction,
        repaint: egui::Context,
    ) {
        let prepared = match action {
            DiscoveryUiAction::Publish {
                target,
                display_alias,
                pin,
                duration_secs,
            } => {
                if duration_secs == 0 {
                    self.share_discovery
                        .command_error("Die Sichtbarkeitsdauer muss positiv sein".into());
                    return;
                }
                if pin.as_bytes().len() > crate::share::DISCOVERY_PIN_MAX_BYTES {
                    self.share_discovery
                        .command_error(pin_limit_error(pin.as_bytes().len()));
                    return;
                }
                if !self.share_discovery.begin_publish(&target) {
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
                if !self.share_discovery.stop_started(&offer_id) {
                    return;
                }
                (
                    DiscoveryCommandContext::Stop(offer_id.clone()),
                    crate::share::ShareCmd::Discovery(
                        crate::share::DiscoveryCommand::StopPublishing { offer_id },
                    ),
                )
            }
            DiscoveryUiAction::Refresh => {
                if self.share_discovery.refreshing {
                    return;
                }
                self.share_discovery.refreshing = true;
                (
                    DiscoveryCommandContext::Refresh,
                    crate::share::ShareCmd::Discovery(
                        crate::share::DiscoveryCommand::ListDiscoveries,
                    ),
                )
            }
            DiscoveryUiAction::Connect { discovery_id, pin } => {
                if pin.as_bytes().len() > crate::share::DISCOVERY_PIN_MAX_BYTES {
                    self.share_discovery
                        .command_error(pin_limit_error(pin.as_bytes().len()));
                    return;
                }
                if !self.share_discovery.connect_started(&discovery_id) {
                    return;
                }
                self.share_discovery.entry_pins.remove(&discovery_id);
                (
                    DiscoveryCommandContext::Connect(discovery_id.clone()),
                    crate::share::ShareCmd::Discovery(
                        crate::share::DiscoveryCommand::StartDiscoveryExchange {
                            discovery_id,
                            pin,
                        },
                    ),
                )
            }
            DiscoveryUiAction::Cancel { exchange_id } => {
                if !self.share_discovery.cancel_started(&exchange_id) {
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
        if let Err((context, error)) = self
            .share_discovery
            .dispatcher
            .submit(prepared.0, prepared.1, repaint)
        {
            self.apply_discovery_command_failure(context, error);
        }
    }

    fn apply_discovery_command_failure(
        &mut self,
        context: DiscoveryCommandContext,
        error: String,
    ) {
        match context {
            DiscoveryCommandContext::Publish(target) => {
                self.share_discovery.publish_command_failed(&target)
            }
            DiscoveryCommandContext::Stop(offer_id) => {
                self.share_discovery.stop_command_failed(&offer_id)
            }
            DiscoveryCommandContext::Refresh => self.share_discovery.refreshing = false,
            DiscoveryCommandContext::Connect(discovery_id) => {
                self.share_discovery.connect_command_failed(&discovery_id)
            }
            DiscoveryCommandContext::Cancel(exchange_id) => {
                self.share_discovery.cancel_command_failed(&exchange_id)
            }
        }
        self.share_discovery.command_error(error);
    }

    pub(in crate::app) fn apply_share_discovery_event(
        &mut self,
        event: crate::share::DiscoveryEvent,
    ) {
        match event {
            crate::share::DiscoveryEvent::OfferPrepared {
                offer_id,
                target,
                display_alias,
                discoverable_until,
            } => self.share_discovery.offer_updated(
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
            } => self.share_discovery.offer_updated(
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
                };
                self.share_discovery.stopped(&offer_id);
                self.share_discovery.status = Some(status.to_string());
            }
            crate::share::DiscoveryEvent::DiscoveryList { advertisements } => {
                let mut entries: Vec<_> = advertisements.into_iter().map(list_entry).collect();
                entries.sort_by(|left, right| {
                    left.display_alias
                        .to_lowercase()
                        .cmp(&right.display_alias.to_lowercase())
                        .then_with(|| left.discovery_id.cmp(&right.discovery_id))
                });
                self.share_discovery.replace_list(entries);
            }
            crate::share::DiscoveryEvent::ExchangeStarted {
                exchange_id,
                discovery_id,
            } => self
                .share_discovery
                .exchange_started(exchange_id, discovery_id),
            crate::share::DiscoveryEvent::ExchangeCompleted {
                exchange_id,
                discovery_id,
                outcome,
            } => self.share_discovery.exchange_completed(
                exchange_id,
                discovery_id,
                outcome_label(outcome),
            ),
            crate::share::DiscoveryEvent::ExchangeCancelled {
                exchange_id,
                discovery_id,
            } => self
                .share_discovery
                .exchange_cancelled(exchange_id, discovery_id),
            crate::share::DiscoveryEvent::ExchangeFailed {
                exchange_id,
                discovery_id,
                error,
            } => self
                .share_discovery
                .exchange_failed(exchange_id, discovery_id, error),
        }
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
