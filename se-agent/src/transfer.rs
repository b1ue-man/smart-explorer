//! The parts of the app's transfer engine that the shared protocol code uses:
//! the compression policy of the LZ4 frames (one definition, included from the
//! app) and the free cores, which the agent does not measure.
#[path = "../../native/src/transfer/core/compress_policy.rs"]
pub(crate) mod compress_policy;

/// The agent has none of the app's sensors: every value is unknown, and the
/// LZ4 frames take the policy's default for an unknown value (one free core).
pub(crate) mod sensors {
    pub(crate) struct Sample;

    impl Sample {
        pub(crate) fn free_cores(&self) -> Option<f64> {
            None
        }
    }

    pub(crate) fn latest() -> Sample {
        Sample
    }
}
