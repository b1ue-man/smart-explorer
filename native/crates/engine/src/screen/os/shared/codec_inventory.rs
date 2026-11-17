//! The codecs of this process (SCR-M3, decision C2 „Kodierer“ and
//! „Dekodierer“): on Windows the Media Foundation transforms found at run
//! time (`MFStartup` and `MFTEnumEx`; without an H.264 encoder there is no
//! Media Foundation entry, nothing is derived from the edition), hardware
//! before software; on Linux the GPU vendor coders found at run time (BK4:
//! NVENC, VA-API per render node; plan 4.10a); on Windows and Linux
//! OpenH264 when the user switched it on; on Linux SVT-AV1 and dav1d where
//! the system has them (plan 2026-11-14 „Bildkette“ BK6, B-25); the
//! lossless tiles always last (`CodecRegistry`). Android has no H.264
//! before SCR-M8.
//!
//! OpenH264 encodes Constrained Baseline up to 3840 × 2160. Its decoder
//! reports `cb` and `high`, because a host never sends B frames, so its
//! `high` is Constrained High (assumption A4, checked by
//! `screen_task_openh264_decodes_constrained_high`). Should that check fail,
//! `OPENH264_DECODES_HIGH` becomes `false` and OpenH264 viewers report `cb`
//! only.
//!
//! Plan 2026-11-17 „Windows-Bild“ P3: a process keeps its register
//! (`registry`) and builds it again only when the OpenH264 switch changes
//! (the host process asked for it with every picture, and every build
//! enumerated and activated the Media Foundation transforms). This user's
//! register runs the run-time checks (`codec_verify.rs`) with the verdicts
//! in `<app data>/screen/codec_verdicts.json`; a register of another store
//! (task tests) checks nothing, as before.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::screen::core::codec_registry::{CodecRegistry, CodecReport, OpenH264Report, VerifyLine};
use crate::screen::core::codec_verdicts::VerdictStore;
use crate::screen::core::codec_verify::Checks;
use crate::screen::core::negotiate::{CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;

/// Assumption A4: OpenH264 decodes High without B frames (Constrained High).
pub const OPENH264_DECODES_HIGH: bool = true;
/// Largest frame of the OpenH264 encoder (`openh264` 0.9.9 refuses more).
pub const OPENH264_MAX_ENCODE: FrameSize = FrameSize::new(3840, 2160);

/// What the OpenH264 encoder offers.
pub fn openh264_encoder_offers() -> Vec<CodecOffer> {
    vec![CodecOffer::h264(H264Profile::ConstrainedBaseline, false).with_max(OPENH264_MAX_ENCODE)]
}

/// What the OpenH264 decoder offers (rule A4 above), up to level 5.2.
pub fn openh264_decoder_offers() -> Vec<CodecOffer> {
    let profiles: &[H264Profile] = if OPENH264_DECODES_HIGH {
        &[H264Profile::High, H264Profile::ConstrainedBaseline]
    } else {
        &[H264Profile::ConstrainedBaseline]
    };
    profiles
        .iter()
        .map(|profile| CodecOffer::h264(*profile, false).with_max(FrameSize::MAX))
        .collect()
}

/// The codecs of this process and the state of OpenH264.
pub struct Inventory {
    pub registry: CodecRegistry,
    pub openh264: OpenH264Report,
}

/// Size of the checks of the whole register (`se screen codec verify`, the
/// warm-up): the 720p size class.
pub const CHECK_SIZE: FrameSize = FrameSize::new(1280, 720);

/// The verdict file of this user.
pub fn verdicts_path() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("screen")
        .join("codec_verdicts.json")
}

/// The checks of this user's codecs, one per process.
fn user_checks() -> Arc<Checks> {
    static CHECKS: OnceLock<Arc<Checks>> = OnceLock::new();
    CHECKS
        .get_or_init(|| Arc::new(Checks::new(VerdictStore::at(verdicts_path()))))
        .clone()
}

/// The codecs with the OpenH264 store `openh264_dir` (`None`: this user's,
/// with the run-time checks). `verify` checks the OpenH264 file against its
/// pin for the report; without it the check runs before every load.
pub fn inventory(openh264_dir: Option<&Path>, verify: bool) -> Inventory {
    let mut found = build(openh264_dir, verify);
    if openh264_dir.is_none() {
        found.registry = found.registry.with_checks(user_checks());
    }
    found
}

fn build(openh264_dir: Option<&Path>, verify: bool) -> Inventory {
    let (mut encoders, mut decoders) = super::media_foundation_codecs();
    let (hardware_encoders, hardware_decoders) = super::hardware_codecs();
    encoders.extend(hardware_encoders);
    decoders.extend(hardware_decoders);
    let (openh264_encoders, openh264_decoders, openh264) =
        super::openh264_codecs(openh264_dir, verify);
    encoders.extend(openh264_encoders);
    decoders.extend(openh264_decoders);
    // BK6 (B-25): software AV1 after the hardware coders of every codec.
    let (av1_encoders, av1_decoders) = super::software_av1_codecs();
    encoders.extend(av1_encoders);
    decoders.extend(av1_decoders);
    Inventory {
        registry: CodecRegistry::new(encoders, decoders),
        openh264,
    }
}

/// The store and OpenH264 switch a kept register was built for.
type Key = (Option<PathBuf>, bool);
/// Most stores a process keeps a register of (the daemon and the host
/// process one; task tests one per store).
const KEPT_STORES: usize = 8;

static KEPT: Mutex<Vec<(Key, CodecRegistry)>> = Mutex::new(Vec::new());

/// The register of the store `openh264_dir` (`None`: this user's), built
/// once per process and again only when the OpenH264 switch changes.
pub fn kept(openh264_dir: Option<&Path>) -> CodecRegistry {
    let switched = super::codec_facade::openh264_store_switched_on(openh264_dir);
    let key: Key = (openh264_dir.map(Path::to_path_buf), switched);
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, registry)) = kept.iter().find(|(known, _)| *known == key) {
        return registry.clone();
    }
    let registry = inventory(openh264_dir, false).registry;
    // The register of the other switch state of this store goes.
    kept.retain(|(known, _)| known.0 != key.0);
    if kept.len() >= KEPT_STORES {
        kept.remove(0);
    }
    kept.push((key, registry.clone()));
    registry
}

/// The codecs of this process and this user.
pub fn registry() -> CodecRegistry {
    kept(None)
}

/// `se screen codec --json` of this process and this user.
pub fn report() -> CodecReport {
    let inventory = inventory(None, true);
    inventory.registry.report(inventory.openh264)
}

/// `se screen codec verify`: forgets the verdicts of this version and
/// checks every encoder of this user at `CHECK_SIZE` (each with the local
/// decoders); the lines of the check and the report afterwards.
pub fn verify() -> (Vec<VerifyLine>, CodecReport) {
    user_checks().store().forget();
    let lines = registry().verify_encoders(CHECK_SIZE, false);
    (lines, report())
}

/// The warm-up of a host (`SessionQuery::VerifyCodecs`): checks the HEVC
/// and AV1 encoders without a verdict, then names the encoder offer.
pub fn warm_up() -> Vec<CodecOffer> {
    let registry = registry();
    registry.verify_encoders(CHECK_SIZE, true);
    registry.encoder_offers()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::core::codec_registry::{CodecEntry, OpenH264State};
    use crate::screen::core::negotiate::{negotiate, CodecChoice, CodecKind, CodecPreference};

    fn tiles_entry() -> CodecEntry {
        CodecEntry {
            codec: CodecKind::Tiles,
            profiles: Vec::new(),
            name: "tiles".into(),
            hardware: false,
            profile_names: Vec::new(),
            refused_settings: Vec::new(),
            verdict: None,
        }
    }

    /// The software AV1 rows of BK6 (present where the system has them).
    fn av1_soft(entry: &CodecEntry) -> bool {
        matches!(entry.name.as_str(), "svt-av1" | "dav1d")
    }

    #[test]
    fn screen_task_codec_inventory_without_mf() {
        // A store without OpenH264: Media Foundation where this host has it,
        // the tiles always. The GitHub Windows runner has no Media
        // Foundation; a build that bound `mfplat.dll` at load time would not
        // even start there.
        let empty = tempfile::tempdir().unwrap();
        let found = inventory(Some(empty.path()), true);
        let report = found.registry.report(found.openh264.clone());
        assert_eq!(report.encoders.last(), Some(&tiles_entry()));
        assert_eq!(report.decoders.last(), Some(&tiles_entry()));
        let others = report.encoders.len() - 1;
        // BK4: a Linux machine with a GPU coder lists it with `hardware`.
        let (gpu_encoders, gpu_decoders) = super::super::hardware_codecs();
        if super::super::media_foundation_present() {
            assert!(
                others > 0,
                "Media Foundation without an encoder row: {report:?}"
            );
            assert!(report.encoders.iter().all(|entry| entry.name != "openh264"));
        } else if !gpu_encoders.is_empty() || !gpu_decoders.is_empty() {
            assert!(report.encoders.iter().all(|entry| entry.name != "openh264"));
            let rows = report.encoders.iter().chain(&report.decoders);
            assert!(rows
                .filter(|entry| entry.codec != CodecKind::Tiles && !av1_soft(entry))
                .all(|entry| entry.hardware));
        } else {
            // BK6: SVT-AV1 and dav1d of the system may stand before the tiles.
            let rows = |list: &[CodecEntry]| -> Vec<CodecEntry> {
                list.iter()
                    .filter(|entry| !av1_soft(entry))
                    .cloned()
                    .collect()
            };
            let offers = |list: Vec<CodecOffer>| -> Vec<CodecOffer> {
                list.into_iter()
                    .filter(|offer| offer.codec != CodecKind::Av1 || offer.hardware)
                    .collect()
            };
            assert_eq!(rows(&report.encoders), vec![tiles_entry()]);
            assert_eq!(rows(&report.decoders), vec![tiles_entry()]);
            assert_eq!(
                offers(found.registry.encoder_offers()),
                vec![CodecOffer::tiles()]
            );
            assert_eq!(
                offers(found.registry.decoder_offers()),
                vec![CodecOffer::tiles()]
            );
        }
        assert_eq!(report.openh264.state, OpenH264State::Absent);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["openh264"]["state"], "absent");
        assert_eq!(json["openh264"]["version"], "2.6.0");
        assert_eq!(
            json["openh264"]["notice"],
            "OpenH264 Video Codec provided by Cisco Systems, Inc."
        );
        let last = json["encoders"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(
            last,
            serde_json::json!({"codec": "tiles", "profiles": [], "name": "tiles", "hardware": false})
        );
        // A host without H.264 still negotiates the tiles with every viewer.
        let choices = negotiate(
            &found.registry.encoder_offers(),
            &openh264_decoder_offers(),
            CodecPreference::Auto,
        );
        assert_eq!(
            choices.last().map(|choice| choice.codec),
            Some(CodecKind::Tiles)
        );
    }
}
