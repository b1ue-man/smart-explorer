//! The signal worker's view of the process power state: low-power
//! transitions, pending probes, CPU holds and the published status.

use std::sync::Arc;

use crate::share::backend::ShareIrohNode;
use crate::share::power::clock::Now;
use crate::share::power::{
    PowerHub, PowerSubscription, ProbeBatch, ProbeOutcome, SignalPowerStatus,
};

/// What changed since the worker last looked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PowerChange {
    pub(crate) entered_low_power: bool,
    pub(crate) left_low_power: bool,
    /// A probe arrived (it waits in `pending` until answered).
    pub(crate) probe: bool,
    pub(crate) network_changed: bool,
}

pub(crate) struct WorkerPower {
    subscription: PowerSubscription,
    low_power: bool,
    pending: Option<ProbeBatch>,
}

impl WorkerPower {
    pub(crate) fn subscribe(iroh: &ShareIrohNode) -> Self {
        Self {
            subscription: iroh.power().subscribe(iroh.signal_wake_sender()),
            low_power: false,
            pending: None,
        }
    }

    pub(crate) fn hub(&self) -> &Arc<PowerHub> {
        self.subscription.hub()
    }

    pub(crate) fn now(&self) -> Now {
        self.hub().now()
    }

    /// Low power as last absorbed; constant between two `absorb` calls.
    pub(crate) fn low_power(&self) -> bool {
        self.low_power
    }

    /// Takes power transitions and new probes. Entering low power and every
    /// probe in low power close quiet peer connections (the CPU is awake
    /// now); a network change goes to Iroh at once.
    pub(crate) fn absorb(&mut self, iroh: &ShareIrohNode) -> PowerChange {
        let mut change = PowerChange::default();
        let low_power = self.hub().low_power();
        if low_power != self.low_power {
            self.low_power = low_power;
            change.entered_low_power = low_power;
            change.left_low_power = !low_power;
        }
        if let Some(batch) = self.subscription.take_probe() {
            change.probe = true;
            change.network_changed = batch.network_changed;
            self.pending
                .get_or_insert_with(ProbeBatch::default)
                .merge(batch);
        }
        if change.network_changed {
            iroh.notify_network_change();
        }
        if self.low_power && (change.entered_low_power || change.probe) {
            iroh.sweep_idle_connections();
        }
        change
    }

    pub(crate) fn probe_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether a pending probe follows a change of the default network.
    pub(crate) fn probe_network_changed(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.network_changed)
    }

    pub(crate) fn answer(&mut self, outcome: ProbeOutcome) {
        if let Some(pending) = self.pending.take() {
            pending.answer(outcome);
        }
    }

    pub(crate) fn hold(&self, hold_ms: u32) {
        self.hub().request_hold(hold_ms);
    }

    pub(crate) fn claim_status(&self, status: SignalPowerStatus) {
        self.subscription.claim_status(status);
    }

    pub(crate) fn update_status(&self, update: impl FnOnce(&mut SignalPowerStatus)) {
        self.subscription.update_status(update);
    }
}
