//! Deflate for the tree data of a peer's analysis (`analysis_deflate_v1`):
//! the host compresses the raw tree encoding, the receiver inflates it back
//! in bounded steps. Stream end, raw length and SHA-256 must all agree.
use std::io::{self, Write};

use flate2::write::DeflateEncoder;
use flate2::{Compression, Decompress, FlushDecompress, Status};

/// Greedy parsing (levels up to 3) keeps a phone host fast; names and sizes
/// already compress several-fold with it.
const LEVEL: u32 = 3;
/// Compressed bytes per data frame, and raw bytes per inflate step (the tree
/// decoder takes at most this much at once).
pub(crate) const STEP_BYTES: usize = 256 * 1024;

/// Compresses the raw encoding into frames of at most `STEP_BYTES`.
pub(crate) struct TreeDeflater {
    encoder: DeflateEncoder<Vec<u8>>,
}

impl TreeDeflater {
    pub(crate) fn new() -> Self {
        Self {
            encoder: DeflateEncoder::new(Vec::new(), Compression::new(LEVEL)),
        }
    }

    /// Compresses `raw`; returns the frames that are complete.
    pub(crate) fn push(&mut self, raw: &[u8]) -> io::Result<Vec<Vec<u8>>> {
        self.encoder.write_all(raw)?;
        if self.encoder.get_ref().len() < STEP_BYTES {
            return Ok(Vec::new());
        }
        Ok(frames(std::mem::take(self.encoder.get_mut())))
    }

    /// The rest of the stream with its final block.
    pub(crate) fn finish(self) -> io::Result<Vec<Vec<u8>>> {
        Ok(frames(self.encoder.finish()?))
    }
}

fn frames(bytes: Vec<u8>) -> Vec<Vec<u8>> {
    if bytes.len() <= STEP_BYTES {
        return if bytes.is_empty() {
            Vec::new()
        } else {
            vec![bytes]
        };
    }
    bytes.chunks(STEP_BYTES).map(<[u8]>::to_vec).collect()
}

/// Inflates frames into raw pieces of at most `STEP_BYTES`, never more than
/// `limit` raw bytes in total (the announced size of the encoding).
pub(crate) struct TreeInflater {
    inflate: Decompress,
    limit: u64,
    ended: bool,
}

impl TreeInflater {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            inflate: Decompress::new(false),
            limit,
            ended: false,
        }
    }

    /// Inflates one compressed frame; every raw piece goes to `sink`.
    pub(crate) fn push(
        &mut self,
        input: &[u8],
        sink: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.ended {
            return Err(invalid("Daten nach dem Ende des komprimierten Ergebnisses"));
        }
        self.run(input, FlushDecompress::None, sink)
    }

    /// Takes what the last frames left buffered.
    pub(crate) fn finish(&mut self, sink: impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
        if self.ended {
            return Ok(());
        }
        self.run(&[], FlushDecompress::Finish, sink)?;
        if !self.ended {
            return Err(invalid("Komprimiertes Ergebnis abgeschnitten"));
        }
        Ok(())
    }

    fn run(
        &mut self,
        mut input: &[u8],
        flush: FlushDecompress,
        mut sink: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut out = Vec::with_capacity(STEP_BYTES);
        loop {
            out.clear();
            let before = self.inflate.total_in();
            let status = self
                .inflate
                .decompress_vec(input, &mut out, flush)
                .map_err(|error| {
                    invalid(&format!("Komprimiertes Ergebnis ist fehlerhaft: {error}"))
                })?;
            let consumed = usize::try_from(self.inflate.total_in() - before)
                .map_err(|_| invalid("Komprimiertes Ergebnis ist fehlerhaft"))?;
            input = &input[consumed.min(input.len())..];
            if self.inflate.total_out() > self.limit {
                return Err(invalid("Analyse-Ergebnis größer als angekündigt"));
            }
            if !out.is_empty() {
                sink(&out)?;
            }
            if status == Status::StreamEnd {
                self.ended = true;
                if !input.is_empty() {
                    return Err(invalid("Daten nach dem Ende des komprimierten Ergebnisses"));
                }
                return Ok(());
            }
            let full = out.len() == out.capacity();
            if !full && (input.is_empty() || (consumed == 0 && out.is_empty())) {
                // Waits for the next frame; a stuck decoder with input left
                // is a broken stream.
                if !input.is_empty() {
                    return Err(invalid("Komprimiertes Ergebnis ist fehlerhaft"));
                }
                return Ok(());
            }
        }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_task_tree_data_survives_deflate_in_bounded_pieces() -> io::Result<()> {
        let raw: Vec<u8> = (0..3 * STEP_BYTES + 17)
            .map(|index| (index % 251) as u8)
            .collect();
        let mut deflater = TreeDeflater::new();
        let mut frames = Vec::new();
        for chunk in raw.chunks(STEP_BYTES) {
            frames.extend(deflater.push(chunk)?);
        }
        frames.extend(deflater.finish()?);
        assert!(frames.iter().all(|frame| frame.len() <= STEP_BYTES));
        let compressed: usize = frames.iter().map(Vec::len).sum();
        assert!(compressed < raw.len() / 4, "{compressed} of {}", raw.len());

        let mut inflater = TreeInflater::new(raw.len() as u64);
        let mut restored = Vec::new();
        for frame in &frames {
            inflater.push(frame, |piece| {
                assert!(!piece.is_empty() && piece.len() <= STEP_BYTES);
                restored.extend_from_slice(piece);
                Ok(())
            })?;
        }
        inflater.finish(|piece| {
            restored.extend_from_slice(piece);
            Ok(())
        })?;
        assert_eq!(restored, raw);

        let mut bomb = TreeInflater::new(1024);
        let error = bomb
            .push(&frames[0], |_| Ok(()))
            .expect_err("more than announced");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        Ok(())
    }
}
