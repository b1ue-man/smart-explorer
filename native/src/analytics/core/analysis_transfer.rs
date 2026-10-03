//! Common completed-result transport for Direct Share and the GUI/worker bridge.
use super::{
    progress::{shorten_tail, MAX_CURRENT_BYTES},
    tree_deflate::{TreeDeflater, TreeInflater},
    tree_transfer::{self, TreeDecoder},
    AnalysisReport, Progress, ScanOutcome, ScanPhase, ScanSnapshot,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum AnalysisMessage {
    Progress { state: ScanSnapshot },
    Ready { report: AnalysisReport },
    Done { sha256: [u8; 32] },
}

/// How a finished outcome is sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SendOptions {
    /// The complete host scan in milliseconds; bridges pass `None` and keep
    /// the evidence they received.
    pub(crate) host_scan_ms: Option<u64>,
    /// `analysis_deflate_v1`: the data frames are one deflate stream.
    pub(crate) deflate: bool,
}

pub(crate) fn send_outcome(
    outcome: &mut ScanOutcome,
    progress: &Progress,
    host_scan_ms: Option<u64>,
    control: impl FnMut(AnalysisMessage) -> io::Result<()>,
    data: impl FnMut(Vec<u8>) -> io::Result<()>,
) -> io::Result<()> {
    let options = SendOptions {
        host_scan_ms,
        deflate: false,
    };
    send_outcome_with(outcome, progress, options, control, data)
}

pub(crate) fn send_outcome_with(
    outcome: &mut ScanOutcome,
    progress: &Progress,
    options: SendOptions,
    mut control: impl FnMut(AnalysisMessage) -> io::Result<()>,
    mut data: impl FnMut(Vec<u8>) -> io::Result<()>,
) -> io::Result<()> {
    progress.check_cancel()?;
    let shape = tree_transfer::shape(outcome.tree.as_ref(), progress)?;
    let mut snapshot = progress.snapshot();
    // Only the exporting worker measures the complete host scan. Bridges pass
    // None and preserve that evidence, including an explicitly unknown value.
    if let Some(scan_ms) = options.host_scan_ms {
        snapshot.host_scan_ms = Some(scan_ms);
    }
    snapshot.current = shorten_tail(&snapshot.current, MAX_CURRENT_BYTES);
    if let Some(tree) = &outcome.tree {
        snapshot.bytes = tree.size;
    }
    let mut report = AnalysisReport::take(outcome, snapshot, shape);
    report.deflate = options.deflate;
    report.fit_wire()?;
    report.validate()?;
    control(AnalysisMessage::Ready { report })?;
    let mut digest = Sha256::new();
    let mut deflater = options.deflate.then(TreeDeflater::new);
    tree_transfer::encode(outcome.tree.take(), progress, |bytes| {
        digest.update(&bytes);
        match &mut deflater {
            Some(deflater) => {
                for frame in deflater.push(&bytes)? {
                    data(frame)?;
                }
                Ok(())
            }
            None => data(bytes),
        }
    })?;
    if let Some(deflater) = deflater {
        for frame in deflater.finish()? {
            data(frame)?;
        }
    }
    control(AnalysisMessage::Done {
        sha256: digest.finalize().into(),
    })
}

#[derive(Default)]
pub(crate) struct AnalysisReceiver {
    report: Option<AnalysisReport>,
    decoder: TreeDecoder,
    digest: Sha256,
    received: u64,
    done: bool,
    inflater: Option<TreeInflater>,
    node_budget: Option<u64>,
}

impl AnalysisReceiver {
    pub(crate) fn with_node_budget(nodes: u64) -> Self {
        Self {
            node_budget: Some(nodes.max(2)),
            ..Self::default()
        }
    }

    pub(crate) fn control(
        &mut self,
        message: AnalysisMessage,
        progress: &Progress,
    ) -> io::Result<Option<ScanOutcome>> {
        progress.check_cancel()?;
        if self.done {
            return Err(invalid("Analyse-Meldung nach Abschluss"));
        }
        match message {
            AnalysisMessage::Progress { state } if self.report.is_none() => {
                progress.receive(state)?
            }
            AnalysisMessage::Ready { report } if self.report.is_none() => {
                report.validate()?;
                if self
                    .node_budget
                    .is_some_and(|limit| report.shape.nodes > limit)
                {
                    return Err(invalid(
                        "Analyse-Ergebnis überschreitet das Knotenbudget des Empfängers",
                    ));
                }
                let memory = report.shape.bytes.saturating_add(
                    report
                        .shape
                        .nodes
                        .saturating_mul(2 * std::mem::size_of::<super::SizeNode>() as u64),
                );
                if self.node_budget.is_some() && memory > crate::transfer::memory_budget() as u64 {
                    return Err(invalid(
                        "Analyse-Ergebnis überschreitet das Speicherbudget des Empfängers",
                    ));
                }
                self.decoder.set_shape(report.shape);
                progress.receive_result(report.progress.clone())?;
                progress.transfer(0, report.shape.bytes);
                self.inflater = report
                    .deflate
                    .then(|| TreeInflater::new(report.shape.bytes));
                self.report = Some(report);
            }
            AnalysisMessage::Done { sha256 } => {
                if let Some(mut inflater) = self.inflater.take() {
                    inflater.finish(|raw| self.raw(raw, progress))?;
                }
                let report = self
                    .report
                    .take()
                    .ok_or_else(|| invalid("Analyse-Ankündigung fehlt"))?;
                if self.received != report.shape.bytes {
                    return Err(invalid("Analyse-Ergebnis abgeschnitten"));
                }
                progress.set_phase(ScanPhase::Verifying, &report.progress.current);
                let actual: [u8; 32] = std::mem::take(&mut self.digest).finalize().into();
                if actual != sha256 {
                    return Err(invalid("Analyse-Ergebnis: SHA-256 stimmt nicht überein"));
                }
                let tree = std::mem::take(&mut self.decoder).finish(report.shape)?;
                let mut final_state = report.progress.clone();
                if let Some(tree) = &tree {
                    final_state.bytes = tree.size;
                }
                progress.receive_result(final_state)?;
                progress.set_phase(ScanPhase::Verifying, &report.progress.current);
                self.done = true;
                return report.finish(tree).map(Some);
            }
            _ => {
                return Err(invalid(
                    "Widersprüchliche Reihenfolge der Analyse-Meldungen",
                ))
            }
        }
        Ok(None)
    }

    pub(crate) fn data(&mut self, bytes: &[u8], progress: &Progress) -> io::Result<()> {
        progress.check_cancel()?;
        if bytes.is_empty() {
            return Err(invalid("Leerer Analyse-Datenblock"));
        }
        if self.report.is_none() || self.done {
            return Err(invalid("Analyse-Daten ohne Ankündigung"));
        }
        match self.inflater.take() {
            Some(mut inflater) => {
                let result = inflater.push(bytes, |raw| self.raw(raw, progress));
                self.inflater = Some(inflater);
                result
            }
            None => self.raw(bytes, progress),
        }
    }

    /// One piece of the raw tree encoding.
    fn raw(&mut self, bytes: &[u8], progress: &Progress) -> io::Result<()> {
        let report = self
            .report
            .as_ref()
            .ok_or_else(|| invalid("Analyse-Daten ohne Ankündigung"))?;
        self.received = self
            .received
            .checked_add(bytes.len() as u64)
            .filter(|n| *n <= report.shape.bytes)
            .ok_or_else(|| invalid("Analyse-Ergebnis zu groß"))?;
        let total = report.shape.bytes;
        self.decoder.push(bytes, progress)?;
        self.digest.update(bytes);
        progress.transfer(self.received, total);
        Ok(())
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "windows_analysis_transfer_task_tests.rs"]
mod task_tests;
