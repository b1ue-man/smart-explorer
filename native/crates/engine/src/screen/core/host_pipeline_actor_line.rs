//! The „Übertragung“ (`delivery.rs`) and the measured rate of the line
//! (`bandwidth_probe.rs`) through the actor: applied by the next `apply`, as
//! the rate ceiling (`set_rate_ceiling`), without an interrupt of the
//! running picture operation; the pacer decides with them from the next
//! picture on.
use super::{Pipeline, PipelineActor};
use crate::screen::core::bandwidth_probe::LineFacts;
use crate::screen::core::delivery::Delivery;

/// What waits for the end of the running picture operation.
#[derive(Default)]
pub(super) struct LinePending {
    delivery: Option<Delivery>,
    measured: Option<u64>,
}

impl LinePending {
    pub(super) fn apply(&mut self, p: &mut Pipeline) {
        if let Some(delivery) = self.delivery.take() {
            p.set_delivery(delivery);
        }
        if let Some(bps) = self.measured.take() {
            p.set_measured_bps(bps);
        }
    }
}

impl PipelineActor {
    /// `SET_DELIVERY` (or the hello's delivery for a running session).
    pub fn set_delivery(&mut self, delivery: Delivery) {
        self.pending.line.delivery = Some(delivery);
        self.poll();
    }

    /// The line's available rate a bandwidth probe found, in bit/s.
    pub fn set_measured_bps(&mut self, bps: u64) {
        self.pending.line.measured = Some(bps);
        self.poll();
    }

    /// The pipeline's „Übertragung“, once applied.
    pub fn delivery(&self) -> Option<Delivery> {
        self.pipeline.as_ref().map(Pipeline::delivery)
    }

    /// The pipeline's measured rate, once applied.
    pub fn measured_bps(&self) -> Option<u64> {
        self.pipeline.as_ref().and_then(Pipeline::measured_bps)
    }

    /// The line facts of the last refresh (`Pipeline::line_facts`).
    pub fn line_facts(&self) -> LineFacts {
        self.facts.line
    }
}
