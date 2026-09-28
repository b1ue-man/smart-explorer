//! Common completed-result transport for Direct Share and the GUI/worker bridge.
use super::{tree_transfer::{self, TreeDecoder}, AnalysisReport, Progress, ScanOutcome, ScanPhase, ScanSnapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum AnalysisMessage {
    Progress { state: ScanSnapshot },
    Ready { report: AnalysisReport },
    Done { sha256: [u8; 32] },
}

pub(crate) fn send_outcome(
    outcome: &mut ScanOutcome, progress: &Progress, scan_ms: u64,
    mut control: impl FnMut(AnalysisMessage) -> io::Result<()>,
    mut data: impl FnMut(Vec<u8>) -> io::Result<()>,
) -> io::Result<()> {
    progress.check_cancel()?;
    let shape = tree_transfer::shape(outcome.tree.as_ref(), progress)?;
    let mut snapshot = progress.snapshot();
    if snapshot.host_scan_ms.is_none() { snapshot.host_scan_ms = Some(scan_ms); }
    let report = AnalysisReport::take(outcome, snapshot, shape);
    report.validate()?;
    control(AnalysisMessage::Ready { report })?;
    let mut digest = Sha256::new();
    tree_transfer::encode(outcome.tree.take(), progress, |bytes| {
        digest.update(&bytes);
        data(bytes)
    })?;
    control(AnalysisMessage::Done { sha256: digest.finalize().into() })
}

#[derive(Default)]
pub(crate) struct AnalysisReceiver {
    report: Option<AnalysisReport>,
    decoder: TreeDecoder,
    digest: Sha256,
    received: u64,
    done: bool,
}

impl AnalysisReceiver {
    pub(crate) fn control(&mut self, message: AnalysisMessage, progress: &Progress) -> io::Result<Option<ScanOutcome>> {
        progress.check_cancel()?;
        if self.done { return Err(invalid("Analyse-Meldung nach Abschluss")); }
        match message {
            AnalysisMessage::Progress { state } if self.report.is_none() => progress.receive(state)?,
            AnalysisMessage::Ready { report } if self.report.is_none() => {
                report.validate()?;
                progress.receive(report.progress.clone())?;
                progress.transfer(0, report.shape.bytes);
                self.report = Some(report);
            }
            AnalysisMessage::Done { sha256 } => {
                let report = self.report.take().ok_or_else(|| invalid("Analyse-Ankündigung fehlt"))?;
                if self.received != report.shape.bytes { return Err(invalid("Analyse-Ergebnis abgeschnitten")); }
                progress.set_phase(ScanPhase::Verifying, &report.progress.current);
                let actual: [u8; 32] = std::mem::take(&mut self.digest).finalize().into();
                if actual != sha256 { return Err(invalid("Analyse-Ergebnis: SHA-256 stimmt nicht überein")); }
                let tree = std::mem::take(&mut self.decoder).finish(report.shape)?;
                self.done = true;
                return report.finish(tree).map(Some);
            }
            _ => return Err(invalid("Widersprüchliche Reihenfolge der Analyse-Meldungen")),
        }
        Ok(None)
    }

    pub(crate) fn data(&mut self, bytes: &[u8], progress: &Progress) -> io::Result<()> {
        let report = self.report.as_ref().filter(|_| !self.done)
            .ok_or_else(|| invalid("Analyse-Daten ohne Ankündigung"))?;
        self.received = self.received.checked_add(bytes.len() as u64)
            .filter(|n| *n <= report.shape.bytes).ok_or_else(|| invalid("Analyse-Ergebnis zu groß"))?;
        self.decoder.push(bytes, progress)?;
        self.digest.update(bytes);
        progress.transfer(self.received, report.shape.bytes);
        Ok(())
    }
}

fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }
