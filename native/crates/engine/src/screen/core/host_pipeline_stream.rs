//! What `STATE` reports of the running picture stream (`StreamInfo`),
//! including the size of the source itself (review 2026-11-19, Codex 4/16).
use super::*;

impl Pipeline {
    /// What `STATE` reports of the running picture stream.
    pub fn stream_info(&self) -> Option<StreamInfo> {
        let (encoder, size) = self.running.as_ref()?;
        Some(StreamInfo {
            codec: self.choice.codec,
            profile: self.choice.profile,
            size: *size,
            encoder: encoder.name.clone(),
            hardware: encoder.hardware,
            decoder_hardware: Some(self.choice.decoder_hardware),
            source_size: self
                .stage
                .as_ref()
                .map(|stage| stage.source.size())
                .filter(|size| size.width > 0 && size.height > 0),
        })
    }
}
