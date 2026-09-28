//! GUI admission into the concurrent transfer lane. The lane, its requests and
//! workers live in `crate::transfer`; they are re-exported here under their
//! previous names.
use super::prelude::*;
use super::*;
pub(in crate::app) use crate::transfer::{
    launch_transfer, FinishedTransfer, TransferLane, TransferRequest,
};

impl App {
    /// Start a transfer at once; transfers on one connection share it
    /// through its flow.
    pub(in crate::app) fn submit_transfer(&mut self, request: TransferRequest) {
        let announcement = request.announcement();
        match self.transfers.submit(request, &mut launch_transfer) {
            Ok(()) => self.notice = Some((announcement, Instant::now())),
            Err(error) => self.error_msg = Some(error),
        }
    }
}
