//! The gate of the run-time codec check (`codec_verify.rs`): at the
//! opening of an encoder without a verdict one round trip runs on its own
//! thread within the hard limit of 3 s; `Failed` skips the encoder like a
//! refusal (`codec_registry_open.rs`), a crash inside the check is caught by
//! the running mark of `codec_verdicts.rs`.
use std::sync::{mpsc, Arc};
use std::time::Duration;

use super::super::codec_registry::{DecoderFactory, EncoderFactory};
use super::super::codec_verdicts::{FailReason, Role, Verdict, VerdictKey, VerdictStore};
use super::super::negotiate::{CodecChoice, CodecKind};
use super::super::pixel_convert::FrameSize;
use super::{round_trip, RealClock, VerifyClock, DEADLINE};

/// The checks of one register: its verdicts and how it measures.
pub struct Checks {
    store: VerdictStore,
    clock: Arc<dyn VerifyClock>,
    deadline: Duration,
}

impl Checks {
    pub fn new(store: VerdictStore) -> Self {
        Self {
            store,
            clock: Arc::new(RealClock::new()),
            deadline: DEADLINE,
        }
    }

    /// The same checks with another clock (task tests).
    pub fn with_clock(mut self, clock: Arc<dyn VerifyClock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn store(&self) -> &VerdictStore {
        &self.store
    }

    pub fn encoder_key(
        factory: &dyn EncoderFactory,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> VerdictKey {
        let device = factory.device();
        VerdictKey::new(
            &factory.name(),
            &device,
            choice.codec,
            choice.profile,
            size,
            Role::Encoder,
        )
    }

    pub fn decoder_key(
        factory: &dyn DecoderFactory,
        codec: CodecKind,
        size: FrameSize,
    ) -> VerdictKey {
        let device = factory.device();
        VerdictKey::new(&factory.name(), &device, codec, None, size, Role::Decoder)
    }

    /// Whether `factory` failed its check for `codec` in every verdict it
    /// has (a decoder never offered, never a reference).
    pub fn decoder_failed(&self, factory: &dyn DecoderFactory, codec: CodecKind) -> bool {
        let summary = self.store.summary(
            &factory.name(),
            &factory.device(),
            codec,
            None,
            Role::Decoder,
        );
        summary.failed.is_some() && !summary.passes()
    }

    /// The gate at the opening of an encoder (`codec_registry_open.rs`):
    /// `Err` with the reason skips it like a refusal; an encoder without a
    /// verdict is checked first.
    pub fn admit(
        &self,
        factory: &Arc<dyn EncoderFactory>,
        decoders: &[Arc<dyn DecoderFactory>],
        choice: &CodecChoice,
        size: FrameSize,
    ) -> Result<(), String> {
        if !choice.codec.is_video() {
            return Ok(());
        }
        let key = Self::encoder_key(factory.as_ref(), choice, size);
        let verdict = match self.store.get(&key) {
            Some(verdict) => Some(verdict),
            None => self.check(factory, decoders, choice, size),
        };
        match verdict {
            Some(Verdict::Failed(reason)) => {
                Err(format!("failed its run-time check ({})", reason.key()))
            }
            _ => Ok(()),
        }
    }

    /// One round trip now, on its own thread within the hard limit; the
    /// verdicts go into the store. `None`: the encoder stays unchecked.
    pub fn check(
        &self,
        factory: &Arc<dyn EncoderFactory>,
        decoders: &[Arc<dyn DecoderFactory>],
        choice: &CodecChoice,
        size: FrameSize,
    ) -> Option<Verdict> {
        let key = Self::encoder_key(factory.as_ref(), choice, size);
        if !self.store.begin(&key) {
            return Some(Verdict::Failed(FailReason::Crashed));
        }
        let skip: Vec<bool> = decoders
            .iter()
            .map(|decoder| self.decoder_failed(decoder.as_ref(), choice.codec))
            .collect();
        let (encoder, list) = (factory.clone(), decoders.to_vec());
        let (choice_copy, clock) = (*choice, self.clock.clone());
        let (sent, answer) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("screen-codec-check".into())
            .spawn(move || {
                let result = round_trip(
                    encoder.as_ref(),
                    &list,
                    &skip,
                    &choice_copy,
                    size,
                    clock.as_ref(),
                );
                let _ = sent.send(result);
            });
        if spawned.is_err() {
            self.store.cancel(&key);
            return None;
        }
        match answer.recv_timeout(self.deadline) {
            Ok(Ok(outcome)) => {
                // Measurement of 2026-11-18: a failed check names its reason
                // in the host log (it ended a session without one).
                if outcome.encoder == Verdict::Structural {
                    eprintln!(
                        "screen codec check of {} at {}x{}: nicht prüfbar (Dekoder: {}); \
                         admitted after the stream checks",
                        factory.name(),
                        size.width,
                        size.height,
                        outcome.detail
                    );
                }
                if let Verdict::Failed(reason) = outcome.encoder {
                    eprintln!(
                        "screen codec check of {} at {}x{}: failed ({}): {}",
                        factory.name(),
                        size.width,
                        size.height,
                        reason.key(),
                        outcome.detail
                    );
                }
                self.store.record(&key, outcome.encoder, &outcome.detail);
                for (index, verdict) in &outcome.decoders {
                    if let Some(decoder) = decoders.get(*index) {
                        let decoder_key = Self::decoder_key(decoder.as_ref(), choice.codec, size);
                        self.store.record(&decoder_key, *verdict, &outcome.detail);
                    }
                }
                Some(outcome.encoder)
            }
            Ok(Err(unchecked)) => {
                eprintln!("screen codec check of {}: {unchecked}", factory.name());
                self.store.cancel(&key);
                None
            }
            Err(_) => {
                let verdict = Verdict::Failed(FailReason::Deadline);
                let detail = format!("no answer within {} ms", self.deadline.as_millis());
                self.store.record(&key, verdict, &detail);
                Some(verdict)
            }
        }
    }
}
