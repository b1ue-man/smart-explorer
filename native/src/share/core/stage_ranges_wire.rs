//! Requests of ranged uploads on the Share (`ranges_v1`): the exclusive
//! stage of one large file, the ranges written into it, and the closing
//! check. They travel as `Ctrl::Fs` requests like all others; a host before
//! `ranges_v1` cannot parse them, so a client sends them only to a host whose
//! Capabilities advertised the feature.
use serde::{Deserialize, Serialize};

use super::is_false;

/// `SegmentedStageOpen`: create the stage of the file `path` (its final path
/// in the export), `size` bytes long. The host picks the stage name itself
/// (`<path>.se-upload-<16 hex from the system's random source>`) and answers
/// `SegmentedStageReady { stage }`; the client never names it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsSegmentedOpen {
    pub(crate) path: String,
    pub(crate) size: u64,
}

/// `StageWriteAt`: the range `[offset, offset + len)` of the open stage
/// `stage`. After the host's `Ready` the client sends exactly `len` bytes in
/// content frames and ends its send stream; the host answers `Ok`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsStageWrite {
    pub(crate) stage: String,
    pub(crate) offset: u64,
    pub(crate) len: u64,
    /// `lz4_v1`: the content may come as LZ4 blocks (`TAG_DATA_LZ4`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) lz4: bool,
}

/// `SegmentedStageFinish`: every range of `stage` has arrived. The host
/// checks the coverage and the length; publication stays `PromoteNoReplace`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FsSegmentedFinish {
    pub(crate) stage: String,
}
