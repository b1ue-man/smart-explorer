//! GUI admission into the concurrent transfer lane. The lane, its requests and
//! workers live in `crate::transfer`; they are re-exported here under their
//! previous names.
use super::prelude::*;
use super::*;
use crate::transfer::Admission;
pub(in crate::app) use crate::transfer::{
    launch_transfer, FinishedTransfer, TransferLane, TransferRequest, MAX_ACTIVE_TRANSFERS,
};

impl App {
    /// Admit a transfer: it starts at once when fewer than
    /// `MAX_ACTIVE_TRANSFERS` run, otherwise it waits in order.
    pub(in crate::app) fn submit_transfer(&mut self, request: TransferRequest) {
        let announcement = request.announcement();
        match self.transfers.submit(request, &mut launch_transfer) {
            Ok(Admission::Started) => {
                self.notice = Some((announcement, Instant::now()));
            }
            Ok(Admission::Queued(position)) => {
                self.notice = Some((
                    format!("{announcement} wartet (Position {position})"),
                    Instant::now(),
                ));
            }
            Err(error) => self.error_msg = Some(error),
        }
    }
}
