use crate::app::app_models::{TransferMsg, TransferProgress};

pub(super) fn send_transfer_progress(
    tx: &crossbeam_channel::Sender<TransferMsg>,
    progress: &TransferProgress,
    last: &mut std::time::Instant,
    force: bool,
) {
    if force || last.elapsed().as_millis() >= 80 {
        let _ = tx.send(TransferMsg::Progress(progress.clone()));
        *last = std::time::Instant::now();
    }
}
