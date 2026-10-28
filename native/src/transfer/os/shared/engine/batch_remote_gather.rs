//! The source's half of a remote → remote packet (G3-13): the members of
//! one `get_batch` collected in the engine's reserved memory, each checked
//! against its listed length (and MD5) before it may reach the target.
use super::super::ops::{Meter, OpError};
use super::super::queue::FileWork;
use super::super::source::RemoteCheck;
use crate::vfs::{BatchSink, FileEvidence};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

/// One member as the source delivered it.
pub(super) enum Slot {
    /// Every listed byte, checked.
    Ready {
        data: Vec<u8>,
        evidence: Option<FileEvidence>,
    },
    /// Read, but not what was listed, or the read failed: nothing exists at
    /// the target.
    Failed(OpError),
    /// The source did not send it.
    NotSent(io::Error),
}

/// The member being received.
struct Current {
    index: usize,
    data: Vec<u8>,
    check: RemoteCheck,
    failure: Option<OpError>,
    /// The source sends another length than listed.
    changed: bool,
    evidence: Option<FileEvidence>,
}

/// Collects the source's packet in memory.
pub(super) struct Gather<'s, 'm> {
    members: &'s [FileWork],
    current: Option<Current>,
    pub(super) slots: Vec<Option<Slot>>,
    /// Evidence announced for the next item (`BatchSink::evidence`).
    evidence: Option<(usize, FileEvidence)>,
    meter: &'s Meter<'m>,
    stop: &'s AtomicBool,
}

impl<'s, 'm> Gather<'s, 'm> {
    pub(super) fn new(members: &'s [FileWork], meter: &'s Meter<'m>, stop: &'s AtomicBool) -> Self {
        Self {
            members,
            current: None,
            slots: (0..members.len()).map(|_| None).collect(),
            evidence: None,
            meter,
            stop,
        }
    }

    fn member(&self, index: usize) -> io::Result<&'s FileWork> {
        self.members
            .get(index)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Paket-Eintrag unbekannt"))
    }

    fn record(&mut self, index: usize, slot: Slot) {
        if let Some(entry) = self.slots.get_mut(index) {
            *entry = Some(slot);
        }
    }
}

impl BatchSink for Gather<'_, '_> {
    fn begin(&mut self, index: usize, size: u64) -> io::Result<()> {
        let file = self.member(index)?;
        let changed = size != file.size;
        let evidence = self
            .evidence
            .take()
            .filter(|(at, _)| *at == index)
            .map(|(_, evidence)| evidence);
        let capacity = if changed {
            0
        } else {
            usize::try_from(size).unwrap_or(0)
        };
        self.current = Some(Current {
            index,
            data: Vec::with_capacity(capacity),
            check: RemoteCheck::new(file, Some(file.size)),
            failure: None,
            changed,
            evidence,
        });
        Ok(())
    }

    fn data(&mut self, index: usize, bytes: &[u8]) -> io::Result<()> {
        if self.stop.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                super::super::super::cancel::CANCELED_ERROR,
            ));
        }
        let file = self.member(index)?;
        let Some(current) = self
            .current
            .as_mut()
            .filter(|current| current.index == index)
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Paket-Daten ohne Beginn",
            ));
        };
        if current.failure.is_some() || current.changed {
            return Ok(());
        }
        let written = current.data.len() as u64;
        if let Err(failure) = current.check.update(file, written, bytes) {
            current.failure = Some(failure);
            current.data = Vec::new();
            return Ok(());
        }
        current.data.extend_from_slice(bytes);
        // The flow sees the work; the progress counts the bytes once they
        // reach the target.
        self.meter.add_hidden(bytes.len() as u64);
        Ok(())
    }

    fn end(&mut self, index: usize, result: io::Result<()>) -> io::Result<()> {
        let current = match self.current.take() {
            Some(current) if current.index == index => current,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Paket-Ende ohne Beginn",
                ))
            }
        };
        let file = self.member(index)?;
        let slot = match (result, current.failure) {
            (_, Some(failure)) => Slot::Failed(failure),
            _ if current.changed => Slot::NotSent(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: Quelle wurde seit der Auflistung geändert", file.source),
            )),
            (Err(error), None) => Slot::Failed(OpError::source(error)),
            (Ok(()), None) => {
                let length = current.data.len() as u64;
                match current.check.finish_streamed(file, length) {
                    Ok(()) => Slot::Ready {
                        data: current.data,
                        evidence: current.evidence,
                    },
                    Err(failure) => Slot::Failed(failure),
                }
            }
        };
        self.record(index, slot);
        Ok(())
    }

    fn failed(&mut self, index: usize, error: io::Error) -> io::Result<()> {
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.index == index)
        {
            self.current = None;
        }
        self.record(index, Slot::NotSent(error));
        Ok(())
    }

    fn evidence(&mut self, index: usize, evidence: FileEvidence) -> io::Result<()> {
        self.evidence = Some((index, evidence));
        Ok(())
    }
}
