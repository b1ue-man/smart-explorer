//! Task suite „Netzrate“ (plan 2026-11-18, all platforms, no network, no
//! GPU): the tiles keep the target E, a failed encoder gives way to another
//! encoder of the same codec first, a chosen codec is never replaced, and an
//! encoder without a first picture counts as failed.
use super::*;
use crate::screen::core::first_picture::{
    FirstPictureWatch, StageAnswer, FIRST_PICTURE_DEADLINE_US,
};
use crate::screen::core::negotiate::{
    negotiate_chosen, preference_named_by, CodecOffer, CodecTrust, H264Profile,
};
use crate::screen::core::rate_control::{PathKind, RateController};
use crate::screen::core::tiles_rate::{TileBudget, TILE_BURST_US};

fn pairing(codec: CodecKind, hardware: bool, max_mbps: Option<u64>) -> CodecChoice {
    CodecChoice {
        codec,
        profile: (codec == CodecKind::H264).then_some(H264Profile::High),
        encoder_hardware: hardware,
        decoder_hardware: true,
        max_size: FrameSize::new(4096, 2304),
        max_pixels: 4096 * 2304,
        max_macroblocks_per_second: max_mbps,
    }
}

/// Bits a sender that offers `frame_bytes` at 30 pictures a second gets
/// through `offer` (admits and books a picture) over `seconds`.
fn sent_bits(seconds: u64, frame_bytes: u64, mut offer: impl FnMut(u64, u64) -> bool) -> u64 {
    let mut bytes = 0;
    let mut now = 0;
    while now < seconds * 1_000_000 {
        if offer(now, frame_bytes) {
            bytes += frame_bytes;
        }
        now += 33_333;
    }
    bytes * 8
}

#[test]
fn netzrate_task_tiles_keep_the_target_and_the_user_ceiling() {
    // 1080p tiles of 200 kB a picture at 30 pictures a second offer
    // 48 Mbit/s (measured: 34–57 Mbit/s); the user set 650 kbit/s.
    const CEILING: u64 = 650_000;
    const SECONDS: u64 = 10;
    let mut budget = TileBudget::default();
    let bits = sent_bits(SECONDS, 200_000, |now, bytes| {
        let admitted = budget.admits(now, CEILING);
        if admitted {
            budget.on_sent(now, bytes, CEILING);
        }
        admitted
    });
    // Within E over the time plus the saved credit and one picture.
    let bound = CEILING * SECONDS + CEILING * TILE_BURST_US / 1_000_000 + 200_000 * 8;
    assert!(bits <= bound, "{bits} > {bound}");
    // The budget does not starve the stream: at least E minus one picture.
    assert!(bits + 200_000 * 8 >= CEILING * SECONDS, "{bits}");
    // A picture larger than the budget is sent whole and moves the next;
    // since the fix „Diashow“ (2026-11-18) the budget is a token bucket
    // that starts with its credit, so the debt is the picture minus it.
    let mut big = TileBudget::default();
    assert!(big.admits(0, CEILING));
    big.on_sent(0, 1_000_000, CEILING);
    assert!(!big.admits(12_000_000, CEILING));
    let debt = 1_000_000 * 8 - CEILING * TILE_BURST_US / 1_000_000;
    assert_eq!(
        big.wait_us(0, CEILING),
        (debt * 1_000_000).div_ceil(CEILING)
    );
    // The rate control keeps running for the tiles: its target E holds the
    // user ceiling, and a lower link estimate (overuse, loss) lowers it.
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_ceiling(Some(CEILING));
    assert!(rate.target_bps() <= CEILING);
    let target = rate.target_bps();
    let bits = sent_bits(SECONDS, 200_000, |now, bytes| {
        let admitted = rate.tiles_admit(now);
        if admitted {
            rate.on_tiles_sent(now, bytes);
        }
        admitted
    });
    let bound = target * SECONDS + target * TILE_BURST_US / 1_000_000 + 200_000 * 8;
    assert!(bits <= bound, "{bits} > {bound}");
    // The pipeline asks the budget before every tile picture and books it
    // when its send starts (fix „Diashow“: not again after the send).
    let produce = include_str!("host_pipeline_produce.rs").replace("\r\n", "\n");
    // Review 2026-11-19: every codec, the video ones at the user's ceiling.
    assert!(produce.contains("if !self.rate.stream_admit(now, video) {"));
    let send = include_str!("host_frame_send.rs");
    assert!(send.contains("self.rate.on_stream_sent(send_started_us, booked, video);"));
    // Back pressure still reaches the rate control for every codec.
    assert!(produce.contains("self.rate.on_back_pressure(now);"));
}

#[test]
fn netzrate_task_failed_encoder_gives_way_to_the_same_codec_first() {
    let h264_hw = pairing(CodecKind::H264, true, None);
    let hevc_hw = pairing(CodecKind::Hevc, true, None);
    // The software MFT carries fewer macroblocks a second than the GPU.
    let h264_sw = pairing(CodecKind::H264, false, Some(245_760));
    let hevc_sw = pairing(CodecKind::Hevc, false, None);
    let choices = [h264_hw, hevc_hw, h264_sw, hevc_sw];
    let fallbacks = codec_fallbacks(&choices, &h264_hw);
    assert_eq!(fallbacks, [h264_sw, hevc_hw, hevc_sw]);
    // An encoder error: the software pairing of the codec comes first; its
    // other limits mean the stage did not try that encoder yet.
    let mut list = fallbacks.clone();
    let next = next_fallback(&mut list, &h264_hw, true, FallbackScope::Automatic);
    assert_eq!(next, Some(h264_sw));
    // The software H.264 failed as well: the next lossy codec, never the
    // tiles while one is left.
    let next = next_fallback(&mut list, &h264_sw, true, FallbackScope::Automatic);
    assert_eq!(next, Some(hevc_hw));
    assert_eq!(
        next_fallback(&mut list, &hevc_hw, true, FallbackScope::Automatic),
        None,
        "the stage tried every HEVC encoder of the same limits"
    );
    // Same limits: the stage already tried the software encoder after an
    // error; a silent encoder was not refused, its software pairing opens
    // the encoders software first.
    let same_sw = pairing(CodecKind::H264, false, None);
    let mut list = codec_fallbacks(&[h264_hw, same_sw, hevc_hw], &h264_hw);
    assert_eq!(list, [same_sw, hevc_hw]);
    let mut silent = list.clone();
    assert_eq!(
        next_fallback(&mut silent, &h264_hw, false, FallbackScope::Automatic),
        Some(same_sw)
    );
    assert_eq!(
        next_fallback(&mut list, &h264_hw, true, FallbackScope::Automatic),
        Some(hevc_hw)
    );
    // The pipeline asks for the next pairing before it takes the tiles.
    let produce = include_str!("host_pipeline_produce.rs");
    let next = produce.find("self.fall_back_to_next_codec()").unwrap();
    let tiles = produce.find("encoder_fallback_to_tiles").unwrap();
    assert!(next < tiles);
    assert!(produce.contains("gave way to"));
}

#[test]
fn netzrate_task_chosen_codec_is_never_replaced() {
    let h264_hw = pairing(CodecKind::H264, true, None);
    let h264_sw = pairing(CodecKind::H264, false, None);
    let hevc_hw = pairing(CodecKind::Hevc, true, None);
    let av1_hw = pairing(CodecKind::Av1, true, None);
    // A chosen codec keeps to its own encoders; „Scharf (GPU)“ to the GPU
    // encoders of H.264 and HEVC; only „Automatisch“ takes the tiles.
    let av1 = FallbackScope::of(CodecPreference::Av1, false);
    assert_eq!(av1, FallbackScope::Codec(CodecKind::Av1));
    assert!(!av1.tiles_allowed());
    let mut list = vec![h264_hw, hevc_hw];
    assert_eq!(next_fallback(&mut list, &av1_hw, true, av1), None);
    assert_eq!(list, [h264_hw, hevc_hw], "other codecs stay unused");
    let sharp = FallbackScope::of(CodecPreference::Auto, true);
    assert_eq!(sharp, FallbackScope::Sharp);
    assert!(!sharp.tiles_allowed());
    let mut list = vec![h264_sw, av1_hw, hevc_hw];
    assert_eq!(
        next_fallback(&mut list, &h264_hw, true, sharp),
        Some(hevc_hw)
    );
    assert!(FallbackScope::of(CodecPreference::Auto, false).tiles_allowed());
    assert!(!FallbackScope::of(CodecPreference::H264, false).tiles_allowed());
    // The negotiation of a chosen codec the host lacks ends with the reason
    // and the codec instead of the tiles.
    let host = [
        CodecOffer::h264(H264Profile::High, true),
        CodecOffer::video(CodecKind::Hevc, false, true),
        CodecOffer::tiles(),
    ];
    // A viewer that chose AV1 names its AV1 decoders only (no tiles).
    let viewer = [CodecOffer::video(CodecKind::Av1, false, true)];
    let refused = negotiate_chosen(&host, &viewer, CodecPreference::Av1, None, CodecTrust::ALL)
        .expect_err("no AV1 encoder");
    assert_eq!(refused.reason, "the host has no encoder of this codec");
    assert!(refused
        .to_string()
        .starts_with("chosen_codec_unavailable: av1: "));
    // With the codec on both sides: its pairings only, no tiles.
    let both = [
        CodecOffer::h264(H264Profile::High, true),
        CodecOffer::video(CodecKind::Hevc, false, true),
        CodecOffer::video(CodecKind::Av1, false, true),
        CodecOffer::tiles(),
    ];
    let chosen =
        negotiate_chosen(&both, &both, CodecPreference::Hevc, None, CodecTrust::ALL).unwrap();
    assert!(!chosen.is_empty());
    assert!(chosen.iter().all(|choice| choice.codec == CodecKind::Hevc));
    let sharp =
        negotiate_chosen(&both, &both, CodecPreference::Sharp, None, CodecTrust::ALL).unwrap();
    assert!(sharp.iter().all(|choice| {
        choice.encoder_hardware && matches!(choice.codec, CodecKind::H264 | CodecKind::Hevc)
    }));
    let auto =
        negotiate_chosen(&both, &both, CodecPreference::Auto, None, CodecTrust::ALL).unwrap();
    assert_eq!(auto.last().map(|c| c.codec), Some(CodecKind::Tiles));
    // A GPU-less host refuses „Scharf (GPU)“ with that reason.
    let cpu = [
        CodecOffer::h264(H264Profile::High, false),
        CodecOffer::tiles(),
    ];
    let refused = negotiate_chosen(&cpu, &both, CodecPreference::Sharp, None, CodecTrust::ALL)
        .expect_err("no GPU encoder");
    assert_eq!(
        refused.reason,
        "the host has no GPU encoder of H.264 or HEVC"
    );
    // The pipeline ends a chosen codec without an encoder with an error.
    let produce = include_str!("host_pipeline_produce.rs");
    assert!(produce.contains("if !self.quality_policy.tiles_allowed() {"));
    assert!(include_str!("host_fallback.rs").contains("chosen_codec_failed: {}"));
}

#[test]
fn netzrate_task_encoder_without_a_first_picture_counts_as_failed() {
    let av1 = pairing(CodecKind::Av1, true, None);
    let mut watch = FirstPictureWatch::default();
    assert!(!watch.on_silent(av1, 0));
    assert!(!watch.on_silent(av1, 1_000_000));
    // Three silent answers, but not yet the deadline.
    assert!(!watch.on_silent(av1, FIRST_PICTURE_DEADLINE_US - 1));
    assert!(watch.on_silent(av1, FIRST_PICTURE_DEADLINE_US));
    // A sent picture or a pause starts the watch again.
    watch.restart();
    assert!(!watch.on_silent(av1, 10 * FIRST_PICTURE_DEADLINE_US));
    // Another pairing is watched from its own first silent answer.
    let mut watch = FirstPictureWatch::default();
    for step in 0..3 {
        watch.on_silent(av1, step * 1_000_000);
    }
    let software = pairing(CodecKind::Av1, false, None);
    assert!(!watch.on_silent(software, FIRST_PICTURE_DEADLINE_US + 1));
    // The pipeline counts empty answers and a new group without a key frame
    // as silent, and a failure goes the fallback order of the scope.
    let produce = include_str!("host_pipeline_produce.rs").replace("\r\n", "\n");
    assert!(produce.contains("self.on_silent_answer(&request, StageAnswer::EncodedNothing)"));
    assert!(produce.contains("self.on_silent_answer(&request, StageAnswer::NoKeyFrame)"));
    assert!(produce.contains("&& !frame.keyframe\n"));
    let fallback = include_str!("host_fallback.rs");
    assert!(fallback.contains("return self.give_up_encoder(error, false);"));
}

#[test]
fn netzrate_task_viewer_choice_reaches_the_host() {
    use crate::screen::core::set_decoders::renegotiate_for;
    let h264 = CodecOffer::h264(H264Profile::High, true);
    let hevc = CodecOffer::video(CodecKind::Hevc, false, true);
    let av1 = CodecOffer::video(CodecKind::Av1, false, true);
    let all = [h264, hevc, av1, CodecOffer::tiles()];
    // The viewer offers no tiles for a chosen codec or „Scharf (GPU)“.
    let kinds = |offers: Vec<CodecOffer>| -> Vec<CodecKind> {
        offers.iter().map(|offer| offer.codec).collect()
    };
    assert_eq!(
        kinds(CodecPreference::Av1.filter_decoders(&all)),
        [CodecKind::Av1]
    );
    assert_eq!(
        kinds(CodecPreference::Sharp.filter_decoders(&all)),
        [CodecKind::H264, CodecKind::Hevc]
    );
    assert_eq!(CodecPreference::Auto.filter_decoders(&all), all);
    assert_eq!(
        kinds(CodecPreference::Tiles.filter_decoders(&all)),
        [CodecKind::Tiles]
    );
    // The host reads the choice from the offers: the tiles mean
    // „Automatisch“ (every older viewer names them).
    for preference in [
        CodecPreference::Auto,
        CodecPreference::H264,
        CodecPreference::Hevc,
        CodecPreference::Av1,
        CodecPreference::Sharp,
    ] {
        let named = preference_named_by(&preference.filter_decoders(&all));
        assert_eq!(named, preference, "{}", preference.key());
    }
    assert_eq!(preference_named_by(&[]), CodecPreference::Auto);
    // `SET_DECODERS` with AV1 chosen at a host without AV1: the reason, no
    // tiles; with H.264 chosen: H.264 only.
    let host = [h264, hevc, CodecOffer::tiles()];
    let refused =
        renegotiate_for(&host, &[av1], None, true, CodecTrust::ALL).expect_err("no AV1 encoder");
    assert!(refused.to_string().contains("av1"));
    let pairing = renegotiate_for(&host, &[h264], None, true, CodecTrust::ALL)
        .unwrap()
        .unwrap();
    assert_eq!(pairing.choice.codec, CodecKind::H264);
    assert_eq!(pairing.preference, CodecPreference::H264);
    assert!(pairing.fallbacks.iter().all(|c| c.codec == CodecKind::H264));
    // The opening negotiates under that choice and refuses without a
    // pairing; `SET_DECODERS` keeps the choice for the fallbacks.
    let start = include_str!("host_server_start.rs");
    assert!(start.contains("let preference = hello.codec_preference();"));
    assert!(start.contains("negotiate::negotiate_chosen("));
    let pairing = include_str!("host_pairing.rs");
    assert!(pairing.contains("self.quality_policy.mode.chosen = next.preference;"));
    assert!(include_str!("host_mode.rs").contains("mode.chosen = previous.chosen;"));
}

#[test]
fn netzrate_task_static_screen_is_no_encoder_failure() {
    // Only an encoder that got a picture and returned none (or no key frame)
    // is blamed; a capture without a picture for the encoder is not.
    assert!(!StageAnswer::NothingCaptured.blames_encoder());
    assert!(StageAnswer::EncodedNothing.blames_encoder());
    assert!(StageAnswer::NoKeyFrame.blames_encoder());
    let produce = include_str!("host_pipeline_produce.rs").replace("\r\n", "\n");
    assert!(produce.contains("Ok(Captured::Unchanged) => Ok(Produced::Idle),"));
    let fallback = include_str!("host_fallback.rs");
    assert!(fallback.contains("if !answer.blames_encoder() {"));
    // A key-frame request on a still screen encodes the last picture again:
    // the source contract and the DXGI capture of Windows.
    let backend = include_str!("host_backend.rs").replace("\r\n", "\n");
    assert!(backend.contains("With `keyframe` a source delivers its last\n    /// picture"));
    let dxgi = include_str!("../os/windows/capture_dxgi.rs");
    assert!(dxgi.contains("(Some(last), true) => SourcePicture::Frame(last.clone()),"));
    // The worker and the in-process stage capture with the key frame as
    // refresh, so the encoder gets that picture.
    let worker = include_str!("../os/shared/host_worker_produce.rs");
    assert!(worker.contains("let refresh = request.keyframe"));
    let capture = include_str!("host_capture.rs").replace("\r\n", "\n");
    // Robust against further reasons inserted between them (9b7a9a43).
    assert!(capture.contains("request.keyframe\n"));
    assert!(capture.contains("|| recover.is_some()"));
}

/// Review 2026-11-19 (R1–R3): „Höchste Datenrate“ is a hard ceiling for the
/// video codecs as for the tiles: every sent byte, key frames included, is
/// booked against it, also below the floor of the target; at the ceiling
/// the session's honour meter tolerates nothing above it.
#[test]
fn review1119_task_video_keeps_the_user_ceiling() {
    use crate::screen::core::rate_control::honour::{HonourMeter, HonourVerdict, HONOUR_WINDOW};
    use crate::screen::core::rate_control::model::frame_budget_bytes;
    use crate::screen::core::rate_control::MIN_BITRATE_BPS;
    const SECONDS: u64 = 10;
    // Without a user ceiling a video stream is not held.
    let mut rate = RateController::new(PathKind::Direct, 0);
    assert_eq!(rate.stream_budget_bps(0, true), None);
    rate.on_stream_sent(0, 10_000_000, true);
    assert!(rate.stream_admit(1, true));
    // A ceiling below the floor stays as given; the target keeps the floor.
    let ceiling = MIN_BITRATE_BPS / 2;
    rate.set_ceiling(Some(ceiling));
    assert_eq!(rate.user_ceiling_bps(), Some(ceiling));
    assert_eq!(rate.target_bps(), MIN_BITRATE_BPS);
    assert_eq!(rate.stream_budget_bps(0, true), Some(ceiling));
    assert_eq!(rate.tiles_budget_bps(0), ceiling);
    // An encoder at 1.4 times the ceiling, every 30th picture a key frame
    // of 20 pictures: the line gets the ceiling over the time plus the
    // saved credit and one key frame, no more.
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.set_ceiling(Some(1_000_000));
    let delta = frame_budget_bytes(1_400_000, 30);
    let (mut bits, mut index, mut now) = (0u64, 0u64, 0u64);
    while now < SECONDS * 1_000_000 {
        if rate.stream_admit(now, true) {
            let bytes = if index % 30 == 0 { delta * 20 } else { delta };
            index += 1;
            rate.on_stream_sent(now, bytes, true);
            bits += bytes * 8;
        }
        now += 33_333;
    }
    let bound = 1_000_000 * SECONDS + 1_000_000 * TILE_BURST_US / 1_000_000 + delta * 20 * 8;
    assert!(bits <= bound, "{bits} > {bound}");
    // Without the budget the same encoder sends well above the ceiling.
    assert!(index < SECONDS * 30, "{index}");
    // The honour meter at the ceiling corrects 1.2 times the budget instead
    // of tolerating it; below the ceiling it is honoured as before.
    let budget = frame_budget_bytes(1_000_000, 30);
    let mut meter = HonourMeter::correcting();
    meter.set_bound(true);
    for _ in 0..HONOUR_WINDOW {
        meter.on_frame(budget * 12 / 10, false, 1_000_000, 1_000_000, 30);
    }
    assert_eq!(meter.verdict(), HonourVerdict::Corrected);
    assert!(meter.commanded_bps(1_000_000) < 1_000_000);
    let mut free = HonourMeter::correcting();
    for _ in 0..HONOUR_WINDOW {
        free.on_frame(budget * 12 / 10, false, 1_000_000, 1_000_000, 30);
    }
    assert_eq!(free.verdict(), HonourVerdict::Honoured);
    // The pipeline asks the budget for every codec and books every picture.
    let produce = include_str!("host_pipeline_produce.rs").replace("\r\n", "\n");
    assert!(produce.contains("if !self.rate.stream_admit(now, video) {"));
    // The control stream's bytes count against the ceiling: the pictures
    // get the rest, never less than the floor; above the whole ceiling the
    // viewer hears it once. Without a ceiling only the statistics count.
    let mut rate = RateController::new(PathKind::Direct, 0);
    rate.on_control_bytes(0, 20_000);
    rate.on_control_bytes(500_000, 20_000);
    assert!(rate.control_bps() > 0);
    assert_eq!(rate.user_picture_bps(), None);
    assert_eq!(rate.control_over_ceiling(), None);
    rate.set_ceiling(Some(2_000_000));
    let pictures = rate.user_picture_bps().unwrap();
    assert!(pictures < 2_000_000 && pictures >= MIN_BITRATE_BPS);
    assert_eq!(rate.stream_budget_bps(0, true), Some(pictures));
    assert!(rate.ceiling_bps() <= pictures);
    // Round 6 L2: below the pictures' floor no picture is made (it would be
    // paced at a few bits a second), and the viewer hears why.
    use crate::screen::core::rate_control::picture_floor_bps;
    assert_eq!(picture_floor_bps(2_000_000), MIN_BITRATE_BPS);
    assert_eq!(picture_floor_bps(100_000), 50_000);
    assert!(!rate.picture_share_short() && rate.stream_admit(0, true));
    rate.set_ceiling(Some(rate.control_bps() + MIN_BITRATE_BPS - 1));
    assert!(rate.picture_share_short());
    assert!(!rate.stream_admit(0, true) && !rate.stream_admit(0, false));
    assert!(rate.user_picture_bps().unwrap() >= MIN_BITRATE_BPS);
    assert!(rate.control_over_ceiling().is_some());
    rate.set_ceiling(Some(2_000_000));
    assert!(rate.control_over_ceiling().is_none());
    rate.set_ceiling(Some(rate.control_bps() / 2));
    assert!(rate.control_over_ceiling().is_some());
    assert_eq!(rate.control_over_ceiling(), None, "once");
    // Small frames and large ones without a ceiling book the control
    // stream's rate; a piece of a large frame under a ceiling the shared
    // budget, before it goes (round 8, Codex 8/5).
    let session = include_str!("host_session_send_write.rs");
    assert!(session.contains("self.pipeline.book_control_bytes(sent);"));
    assert!(session.contains("None => self.pipeline.book_control_bytes(len as u64),"));
    assert!(session.contains(".on_held_sent(now_us(), len as u64, rate)"));
    // Codex 5/4, round 6 L3: the session's sound counts with the control
    // stream as the rate between two readings of the link's running total.
    let mut sound = RateController::new(PathKind::Direct, 0);
    sound.set_ceiling(Some(2_000_000));
    assert_eq!(sound.on_audio_total(0, 0), 0);
    assert_eq!(sound.on_audio_total(1_000_000, 40_000), 40_000);
    assert_eq!(sound.control_bps(), 320_000);
    assert_eq!(sound.user_picture_bps(), Some(1_680_000));
    // The next second without sound: no rate is left over.
    assert_eq!(sound.on_audio_total(2_000_000, 40_000), 0);
    assert_eq!(sound.control_bps(), 0);
    let link = include_str!("../os/shared/audio_host.rs");
    assert!(link.contains("sent.fetch_add(bytes, Ordering::AcqRel);"));
}
