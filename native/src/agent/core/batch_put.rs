//! Batch upload over the agent protocol (`batch-v1`, only with `credit-v1`).
//! Entries stream one after another from `data` without buffering; each gets
//! a trailer, so a failed or short source read discards just that entry.
//! After such a failure the byte stream no longer lines up with the entries,
//! so the rest of the batch is not sent and fails for certain. Outcomes the
//! server did not report because the connection broke stay unknown (`Err`).
use super::agent_error::agent_error;
use super::backend::AgentBackend;
use super::mux::Mux;
use super::route::RequestRx;
use crate::agent_proto::{
    put_entry_len, split_batch, BatchEntry, Frame, BATCH_UNKNOWN_MARKER, CHUNK,
};
use crate::vfs::{BatchPut, BatchPutOutcome};
use crossbeam_channel::TryRecvError;
use std::io::{self, Read};

/// Outcome slots of one `put_batch`, `None` until known.
type Outcomes = [Option<BatchPutOutcome>];

fn protocol(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

fn nonce_base() -> io::Result<u64> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn not_sent(stopped: Option<usize>, index: usize) -> io::Error {
    let reason = match stopped {
        Some(failed) => format!(
            "Paket nach dem Lesefehler bei Eintrag {} beendet",
            failed + 1
        ),
        None => "Paket vorzeitig beendet".to_string(),
    };
    // Nothing of this entry reached the server: safe to transfer again.
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!("Nicht übertragen (Eintrag {}): {reason}", index + 1),
    )
}

/// Record one reported outcome; an outcome the client already knows (its
/// own read failure) is kept.
fn record(outcomes: &mut Outcomes, offset: usize, len: usize, frame: Frame) -> io::Result<()> {
    let (index, outcome) = match frame {
        Frame::ItemPublished { index, path } => (index, BatchPutOutcome::Published(path)),
        Frame::ItemFailed { index, message } => {
            (index, BatchPutOutcome::Failed(agent_error(message)))
        }
        _ => return Err(protocol("unerwartete Paket-Antwort")),
    };
    let local = index as usize;
    if local >= len {
        return Err(protocol("Paket-Antwort mit ungültigem Index"));
    }
    let slot = &mut outcomes[offset + local];
    if slot.is_none() {
        *slot = Some(outcome);
    }
    Ok(())
}

enum Ended {
    Open,
    Failed(String),
}

/// Take outcomes that already arrived, without waiting.
fn drain(rx: &RequestRx, outcomes: &mut Outcomes, offset: usize, len: usize) -> io::Result<Ended> {
    loop {
        match rx.try_recv() {
            Ok(Frame::Err(message)) => return Ok(Ended::Failed(message)),
            Ok(frame @ (Frame::ItemPublished { .. } | Frame::ItemFailed { .. })) => {
                record(outcomes, offset, len, frame)?
            }
            Ok(_) => return Err(protocol("unerwartete Paket-Antwort")),
            Err(TryRecvError::Empty) => return Ok(Ended::Open),
            Err(TryRecvError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Agent-Verbindung während des Pakets geschlossen",
                ))
            }
        }
    }
}

/// Stream exactly `size` bytes of one entry. `Ok(Some)` = the source failed
/// or ended early (only this entry is lost); `Err` = the connection failed.
fn stream_entry(
    mux: &Mux,
    id: u64,
    size: u64,
    data: &mut dyn Read,
    buffer: &mut [u8],
) -> io::Result<Option<io::Error>> {
    let mut remaining = size;
    while remaining > 0 {
        let want = remaining.min(buffer.len() as u64) as usize;
        let read = match data.read(&mut buffer[..want]) {
            Ok(0) => {
                return Ok(Some(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Quelle endete vor der angekündigten Länge",
                )))
            }
            Ok(read) => read,
            Err(error) => return Ok(Some(error)),
        };
        mux.send(id, Frame::Data(buffer[..read].to_vec()))?;
        remaining -= read as u64;
    }
    Ok(None)
}

impl AgentBackend {
    pub(super) fn agent_put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> io::Result<Vec<BatchPutOutcome>> {
        if !self.features().batches() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "batch upload not supported",
            ));
        }
        let base = nonce_base()?;
        let wire: Vec<BatchEntry> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| BatchEntry {
                path: entry.path.clone(),
                size: entry.size,
                nonce: base.wrapping_add(index as u64),
            })
            .collect();
        let header: Vec<usize> = wire.iter().map(put_entry_len).collect();
        let sizes: Vec<u64> = wire.iter().map(|entry| entry.size).collect();
        let mut outcomes: Vec<Option<BatchPutOutcome>> = entries.iter().map(|_| None).collect();
        let mut stopped = None;
        for range in split_batch(&header, &sizes) {
            stopped =
                self.put_wire_batch(&wire[range.clone()], range.start, data, &mut outcomes)?;
            if stopped.is_some() {
                break;
            }
        }
        Ok(outcomes
            .into_iter()
            .enumerate()
            .map(|(index, outcome)| {
                outcome.unwrap_or_else(|| BatchPutOutcome::Failed(not_sent(stopped, index)))
            })
            .collect())
    }

    /// One wire batch; returns the global index of an entry whose source
    /// read failed (the batch stopped there).
    fn put_wire_batch(
        &self,
        wire: &[BatchEntry],
        offset: usize,
        data: &mut dyn Read,
        outcomes: &mut Outcomes,
    ) -> io::Result<Option<usize>> {
        let lease = self.pool.lease();
        let mux = lease.mutation_mux()?;
        let (id, rx) = mux.register();
        let result = (|| {
            mux.send(
                id,
                Frame::BatchPut {
                    entries: wire.to_vec(),
                },
            )?;
            let mut buffer = vec![0u8; CHUNK];
            let mut sent = 0;
            let mut stopped = None;
            let mut ended = Ended::Open;
            for (local, entry) in wire.iter().enumerate() {
                let index = u32::try_from(local).map_err(|_| protocol("Paket zu groß"))?;
                let failure = match stream_entry(&mux, id, entry.size, data, &mut buffer) {
                    Ok(failure) => failure,
                    // The server may have ended the request (its reply
                    // closes our credit): report its reason, not ours.
                    Err(error) => match drain(&rx, outcomes, offset, wire.len())? {
                        Ended::Failed(message) => {
                            ended = Ended::Failed(message);
                            break;
                        }
                        Ended::Open => return Err(error),
                    },
                };
                mux.send(
                    id,
                    Frame::ItemEnd {
                        index,
                        error: failure.as_ref().map(ToString::to_string),
                    },
                )?;
                sent = local + 1;
                if let Some(error) = failure {
                    outcomes[offset + local] = Some(BatchPutOutcome::Failed(error));
                    stopped = Some(offset + local);
                    break;
                }
                if let Ended::Failed(message) = drain(&rx, outcomes, offset, wire.len())? {
                    ended = Ended::Failed(message);
                    break;
                }
            }
            if matches!(ended, Ended::Open) {
                mux.send(id, Frame::End)?;
                ended = finish(&rx, outcomes, offset, wire.len())?;
            }
            if let Ended::Failed(message) = &ended {
                if let Some(reason) = message.strip_prefix(BATCH_UNKNOWN_MARKER) {
                    // The service's peer failed mid-batch: outcome unknown.
                    return Err(agent_error(reason.trim_start().to_string()));
                }
            }
            if let Ended::Failed(message) = ended {
                // A refused or failed batch published nothing beyond the
                // outcomes it reported before its error.
                for slot in &mut outcomes[offset..offset + wire.len()] {
                    if slot.is_none() {
                        *slot = Some(BatchPutOutcome::Failed(agent_error(message.clone())));
                    }
                }
                return Ok(stopped);
            }
            if outcomes[offset..offset + sent].iter().any(Option::is_none) {
                return Err(protocol("Paket-Antwort ohne Ergebnis für jeden Eintrag"));
            }
            Ok(stopped)
        })();
        if matches!(&result, Err(error) if error.kind() == io::ErrorKind::InvalidData) {
            lease.invalidate(&mux);
        }
        mux.unregister(id);
        result
    }
}

/// Wait for the remaining outcomes and the final reply.
fn finish(rx: &RequestRx, outcomes: &mut Outcomes, offset: usize, len: usize) -> io::Result<Ended> {
    loop {
        match rx.recv() {
            Ok(Frame::Ok) => return Ok(Ended::Open),
            Ok(Frame::Err(message)) => return Ok(Ended::Failed(message)),
            Ok(frame @ (Frame::ItemPublished { .. } | Frame::ItemFailed { .. })) => {
                record(outcomes, offset, len, frame)?
            }
            Ok(_) => return Err(protocol("unerwartete Paket-Antwort")),
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Agent-Verbindung während des Pakets geschlossen",
                ))
            }
        }
    }
}
