//! „Automatisch“ without AV1 first and the mode „Scharf (GPU)“ in the
//! register (plan 2026-11-17 P9): which checked codecs keep their B-30 rank
//! (`CodecTrust`), and which rows of `se screen codec` the mode uses.
use super::{CodecEntry, CodecKind, CodecReport, CodecTrust, H264Profile};

/// Whether a checked `codec` keeps its B-30 rank in „Automatisch“: every
/// codec but AV1. User decision 2026-11-17 („av1 ist zu viel. punkt.“, plan
/// P9): AV1 kept the first rank of the hardware/hardware tier on AMD and
/// filled the path under continuous motion; it stays selectable by name
/// („AV1“), and without trust it follows H.264 and HEVC (`CodecTrust::rank`).
pub fn keeps_first_rank(codec: CodecKind) -> bool {
    codec != CodecKind::Av1
}

/// The `CodecTrust` of the checked (or offered) `codecs` for the
/// negotiation of „Automatisch“: each of them that `keeps_first_rank`.
pub fn first_rank_trust(codecs: impl IntoIterator<Item = CodecKind>) -> CodecTrust {
    codecs
        .into_iter()
        .filter(|codec| keeps_first_rank(*codec))
        .fold(CodecTrust::NONE, CodecTrust::with)
}

impl CodecReport {
    /// The encoders of mode „Scharf (GPU)“ (`sharp_mode.rs`, plan P9):
    /// hardware H.264 and HEVC rows whose run-time check did not fail.
    pub fn sharp_encoders(&self) -> Vec<&CodecEntry> {
        self.encoders
            .iter()
            .filter(|entry| {
                entry.hardware && crate::screen::core::sharp_mode::sharp_codec(entry.codec)
            })
            .filter(|entry| {
                entry
                    .verdict
                    .as_ref()
                    .is_none_or(|verdict| verdict.state != "failed")
            })
            .collect()
    }

    /// Whether one of `sharp_encoders` encodes 4:4:4 (HEVC `main_444`).
    pub fn sharp_444(&self) -> bool {
        self.sharp_encoders().iter().any(|entry| {
            entry
                .profile_names
                .iter()
                .any(|name| name == entry.codec.profile_name(Some(H264Profile::High)))
                && entry.codec == CodecKind::Hevc
        })
    }
}
