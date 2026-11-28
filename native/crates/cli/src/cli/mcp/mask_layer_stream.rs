//! One output stream of a job and its masking (plan 2026-11-28, contract
//! 4): a [`StreamMode::Raw`] stream (command output) runs through a stream
//! masker of the active mask lists before it reaches its spool, so a value
//! split across two appends is still found and every page of `job_status`
//! comes from the masked stream; a [`StreamMode::PreMasked`] stream (search
//! hits, grep lines, replace records) is masked by the tool that writes it.
use super::mask_layer::MaskLayer;
use super::spool::Spool;
use crate::masks::{MaskSet, StreamMasker};
use std::sync::{Arc, Mutex};

/// How the output of a stream is masked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StreamMode {
    /// Bytes of a command or program: masked while they are appended.
    Raw,
    /// Records the tool masked itself.
    PreMasked,
}

impl StreamMode {
    /// The mode of a stream created by name (`JobRegistry::create`):
    /// `stdout` and `stderr` carry command output, every other stream
    /// (`hits`, `matches`, …) records its tool writes masked.
    pub(super) fn of(name: &str) -> Self {
        match name {
            "stdout" | "stderr" => Self::Raw,
            _ => Self::PreMasked,
        }
    }
}

pub(super) struct Stream {
    pub(super) name: &'static str,
    pub(super) spool: Spool,
    /// The stream masker of a raw stream of a session with mask lists.
    mask: Option<Mutex<StreamMask>>,
}

impl Stream {
    /// `masked`: the job belongs to a session with mask lists.
    pub(super) fn new(name: &'static str, spool: Spool, mode: StreamMode, masked: bool) -> Self {
        Self {
            name,
            spool,
            mask: (mode == StreamMode::Raw && masked).then(|| Mutex::new(StreamMask::default())),
        }
    }

    /// Masks (a raw stream) and keeps `bytes`.
    pub(super) fn append(
        &self,
        layer: Option<&Arc<MaskLayer>>,
        bytes: &[u8],
    ) -> Result<(), String> {
        let masked;
        let bytes = match (&self.mask, layer) {
            (Some(mask), Some(layer)) => {
                masked = lock(mask)?.push(layer, bytes)?;
                masked.as_slice()
            }
            (Some(_), None) => return Err("job output masker without mask lists".into()),
            (None, _) => bytes,
        };
        self.keep(bytes)
    }

    /// Keeps what the masker held back once the stream was silent; returns
    /// whether there was any.
    pub(super) fn release_idle(&self) -> Result<bool, String> {
        let Some(mask) = &self.mask else {
            return Ok(false);
        };
        let rest = lock(mask)?.release_idle();
        self.keep(&rest)?;
        Ok(!rest.is_empty())
    }

    /// Keeps everything the masker still holds (end of the job).
    pub(super) fn flush(&self) -> Result<(), String> {
        let Some(mask) = &self.mask else {
            return Ok(());
        };
        let rest = lock(mask)?.flush();
        self.keep(&rest)
    }

    fn keep(&self, bytes: &[u8]) -> Result<(), String> {
        self.spool
            .append(bytes)
            .map_err(|error| format!("cannot keep job output: {error}"))
    }
}

fn lock(mask: &Mutex<StreamMask>) -> Result<std::sync::MutexGuard<'_, StreamMask>, String> {
    mask.lock()
        .map_err(|_| "job output masker is poisoned".to_string())
}

/// The stream masker of one raw job stream. It follows the active lists:
/// when they change, what the old masker holds is written and a new one
/// continues with the new lists.
#[derive(Default)]
struct StreamMask {
    current: Option<(Arc<MaskSet>, StreamMasker)>,
}

impl StreamMask {
    /// The masked bytes ready for the spool.
    fn push(&mut self, layer: &MaskLayer, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let set = layer.set()?;
        let mut out = Vec::new();
        let same = self
            .current
            .as_ref()
            .is_some_and(|(current, _)| Arc::ptr_eq(current, &set));
        if !same {
            out.extend(self.flush());
            if !set.is_empty() {
                let masker = set.stream();
                self.current = Some((set, masker));
            }
        }
        match self.current.as_mut() {
            Some((_, masker)) => out.extend(masker.push(bytes)),
            None => out.extend_from_slice(bytes),
        }
        Ok(out)
    }

    /// What the masker held back once the stream was silent for
    /// `STREAM_IDLE_RELEASE` (a prompt that waits for input), else nothing.
    fn release_idle(&mut self) -> Vec<u8> {
        match self.current.as_mut() {
            Some((_, masker)) => masker.release_idle(),
            None => Vec::new(),
        }
    }

    /// Everything the masker still holds; the stream continues unmasked
    /// until the next push reads the lists again.
    fn flush(&mut self) -> Vec<u8> {
        match self.current.take() {
            Some((_, mut masker)) => masker.flush(),
            None => Vec::new(),
        }
    }
}
