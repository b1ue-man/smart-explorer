//! Batch download over the agent protocol (`batch-v1`, only with
//! `credit-v1`): items arrive in request order straight into the caller's
//! sink; an error from the sink cancels the rest of the batch.
use super::agent_error::agent_error;
use super::backend::AgentBackend;
use crate::agent_proto::{get_item_len, split_batch, BatchItem, Frame};
use crate::vfs::{BatchGet, BatchSink};
use std::io;

fn protocol(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// The item currently receiving bytes.
struct Open {
    local: usize,
    size: u64,
    received: u64,
}

struct Receiving<'a> {
    sink: &'a mut dyn BatchSink,
    offset: usize,
    len: usize,
    reported: Vec<bool>,
    open: Option<Open>,
}

impl Receiving<'_> {
    fn local(&self, index: u32) -> io::Result<usize> {
        let local = index as usize;
        if local >= self.len || self.reported[local] {
            return Err(protocol("Paket-Antwort mit ungültigem Index"));
        }
        Ok(local)
    }

    /// Handle one frame; `Ok(true)` once the batch ended.
    fn accept(&mut self, frame: Frame) -> io::Result<bool> {
        match frame {
            Frame::ItemBegin { index, size } => {
                let local = self.local(index)?;
                if self.open.is_some() {
                    return Err(protocol("Paket-Eintrag begann vor dem Ende des vorigen"));
                }
                self.sink.begin(self.offset + local, size)?;
                self.open = Some(Open {
                    local,
                    size,
                    received: 0,
                });
            }
            Frame::Data(bytes) => {
                let open = self
                    .open
                    .as_mut()
                    .ok_or_else(|| protocol("Paket-Daten ohne Eintrag"))?;
                open.received = open
                    .received
                    .checked_add(bytes.len() as u64)
                    .filter(|received| *received <= open.size)
                    .ok_or_else(|| protocol("Paket-Eintrag überschreitet seine Länge"))?;
                let index = self.offset + open.local;
                self.sink.data(index, &bytes)?;
            }
            Frame::ItemEnd { index, error } => {
                let local = self.local(index)?;
                let open = self
                    .open
                    .take()
                    .filter(|open| open.local == local)
                    .ok_or_else(|| protocol("Paket-Ende ohne passenden Eintrag"))?;
                if error.is_none() && open.received != open.size {
                    return Err(protocol("Paket-Eintrag endete vor seiner Länge"));
                }
                self.reported[local] = true;
                self.sink.end(
                    self.offset + local,
                    error.map_or(Ok(()), |m| Err(agent_error(m))),
                )?;
            }
            Frame::ItemFailed { index, message } => {
                let local = self.local(index)?;
                if self.open.is_some() {
                    return Err(protocol("Paket-Fehler mitten in einem Eintrag"));
                }
                self.reported[local] = true;
                self.sink
                    .failed(self.offset + local, agent_error(message))?;
            }
            Frame::End => {
                if self.open.is_some() {
                    return Err(protocol("Paket endete mitten in einem Eintrag"));
                }
                return Ok(true);
            }
            Frame::Err(message) => return Err(agent_error(message)),
            other => return Err(protocol(&format!("unerwartete Paket-Antwort: {other:?}"))),
        }
        Ok(false)
    }

    /// The batch failed: an item that already began gets its error, so the
    /// sink discards the bytes it received.
    fn abort_open(&mut self, error: &io::Error) {
        if let Some(open) = self.open.take() {
            let _ = self.sink.end(
                self.offset + open.local,
                Err(io::Error::new(error.kind(), error.to_string())),
            );
        }
    }
}

impl AgentBackend {
    pub(super) fn agent_get_batch(
        &self,
        items: &[BatchGet],
        sink: &mut dyn BatchSink,
    ) -> io::Result<()> {
        if !self.features().batches() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "batch download not supported",
            ));
        }
        let wire: Vec<BatchItem> = items
            .iter()
            .map(|item| BatchItem {
                path: item.path.clone(),
                id: item.id.clone(),
                size: item.size,
            })
            .collect();
        let header: Vec<usize> = wire.iter().map(get_item_len).collect();
        let sizes: Vec<u64> = wire.iter().map(|item| item.size).collect();
        for range in split_batch(&header, &sizes) {
            self.get_wire_batch(&wire[range.clone()], range.start, sink)?;
        }
        Ok(())
    }

    fn get_wire_batch(
        &self,
        wire: &[BatchItem],
        offset: usize,
        sink: &mut dyn BatchSink,
    ) -> io::Result<()> {
        let lease = self.pool.lease();
        let mux = lease.mux()?;
        let (id, rx) = mux.register();
        let mut receiving = Receiving {
            sink,
            offset,
            len: wire.len(),
            reported: vec![false; wire.len()],
            open: None,
        };
        let result = (|| {
            mux.send(
                id,
                Frame::BatchGet {
                    items: wire.to_vec(),
                },
            )?;
            loop {
                let frame = rx.recv().map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Agent-Verbindung während des Pakets geschlossen",
                    )
                })?;
                if receiving.accept(frame)? {
                    break;
                }
            }
            // Every item gets an answer, even from a server that skipped one.
            let Receiving { sink, reported, .. } = &mut receiving;
            for (local, done) in reported.iter_mut().enumerate() {
                if !*done {
                    *done = true;
                    sink.failed(offset + local, protocol("keine Antwort für diese Datei"))?;
                }
            }
            Ok(())
        })();
        if let Err(error) = &result {
            // Unregistering cancels the request; frames still in flight are
            // dropped, so the connection itself stays usable.
            receiving.abort_open(error);
        }
        mux.unregister(id);
        result
    }
}
