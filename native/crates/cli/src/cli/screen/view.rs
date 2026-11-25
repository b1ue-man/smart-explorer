//! `se screen view DEVICE [--source QUELLE] [--control] [--fullscreen]
//! [--max-rate RATE] [--codec CODEC] [--decoder DECODER]
//! [--decoder-library LIBRARY] [--display DISPLAY]
//! [--virtual-display auto|off|request] [--display-size WxH|window]
//! [--audio on|off] [--transport realtime|reliable] [--json]`
//! and `se screen grid [--room RAUM] [--json]` (SCR-M7): open the desktop
//! viewer window of the GUI program (`--screen-view <ziel-json>`,
//! `--screen-grid [--room <id>]`, V5) as a process of its own and return.
//! The window asks the device and its user itself. Output: `launched\t<pid>`
//! or, with `--json`, `{"launched":true,"pid":<n>}`. Without the desktop app
//! next to this `se` the command fails with „Desktop-App nicht installiert“.
//! `--transport` stores „Übertragung“ (user 2026-11-25): `realtime`
//! („Echtzeit (wie UDP)“, never chosen) or `reliable` („Zuverlässig (wie
//! TCP)“); the window opens every session with it.
//! „Monitor“ and „Auflösung“ changed later in the window's toolbar apply
//! within the running session at a host that answers `live` (P9,
//! `SET_DISPLAY`); an older host's session opens again with them.
//!
//! User rule 2026-11-24 („was du anzeigst MUSS funktionieren“): a `--codec`
//! or `--decoder` that the device and this one cannot run together is
//! refused before anything is stored, with the German reason and the codecs
//! they can run (`served_check`; exit code non-zero).
use clap::Args;
use clap_complete::ArgValueCandidates;
use serde_json::{json, Value};

use super::DeviceChoice;
use crate::app::screen_delivery::Delivery;
use crate::app::screen_prefs::DisplayChoice;
use crate::app::start_request::StartRequest;
use crate::screen::core::decoder_library::DecoderLibrary;
use crate::screen::core::negotiate::{CodecSetting, DecoderPreference};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::virtual_display::{parse_size, VirtualDisplayWish};
use crate::screen::core::window_list::SourceId;

#[derive(Args, Debug, PartialEq, Eq)]
pub(super) struct ViewArgs {
    #[arg(
        help = "Device: share:// endpoint, name, id or fingerprint prefix",
        add = ArgValueCandidates::new(crate::cli::completions_screen::device_candidates)
    )]
    device: String,
    /// Source: monitor:<id> or window:<id> (default: the whole screen)
    #[arg(long, value_name = "QUELLE")]
    source: Option<String>,
    /// Start with control („Bildschirm steuern“)
    #[arg(long)]
    control: bool,
    /// Open in full screen
    #[arg(long)]
    fullscreen: bool,
    /// „Höchste Datenrate“ of the desktop viewer, stored before it opens:
    /// `auto`, or bit/s with an optional SI prefix (`k`, `M`, `G`), e.g. `20M`
    #[arg(long, value_name = "RATE", value_parser = parse_max_rate)]
    max_rate: Option<MaxRate>,
    /// „Codec“ of the desktop viewer, stored before it opens: auto, h264,
    /// hevc, av1, sharp (GPU, P9) or lossless (plan 2026-11-14 „Bildkette“ 4.10)
    #[arg(long, value_name = "CODEC", value_parser = parse_codec)]
    codec: Option<CodecSetting>,
    /// „Dekodierer“ of the desktop viewer, stored before it opens: auto,
    /// hardware or software
    #[arg(long, value_name = "DECODER", value_parser = parse_decoder)]
    decoder: Option<DecoderPreference>,
    /// „Dekodierer-Bibliothek“ (Windows), stored before the viewer opens:
    /// auto (FFmpeg, then Media Foundation), ffmpeg or media_foundation
    #[arg(long, value_name = "LIBRARY", value_parser = parse_library)]
    decoder_library: Option<DecoderLibrary>,
    /// „Anzeige“ (Windows), stored before the viewer opens: auto,
    /// direct3d11 or wgpu
    #[arg(long, value_name = "DISPLAY", value_parser = parse_display)]
    display: Option<DisplayChoice>,
    /// „Monitor“ of the desktop viewer, stored before it opens: auto (se's
    /// own display only when the device has no monitor), off (the physical
    /// monitor as it is) or request (se's own display also at a monitor)
    #[arg(long, value_name = "WAHL", value_parser = parse_virtual_display)]
    virtual_display: Option<VirtualDisplayWish>,
    /// „Auflösung“ of se's own display (with off or at a monitor: the mode
    /// the physical monitor lists nearest to it), stored before the viewer
    /// opens: WxH (e.g. 2560x1440) or window (follow the viewer's window)
    #[arg(long, value_name = "GRÖSSE", value_parser = parse_display_size)]
    display_size: Option<DisplaySize>,
    /// „Ton“ of the desktop viewer, stored before it opens: on (the
    /// device's sound plays in the viewer, the default) or off
    #[arg(long, value_name = "AN_AUS", value_parser = parse_audio)]
    audio: Option<bool>,
    /// „Übertragung“ of the desktop viewer, stored before it opens: realtime
    /// (late pictures are given up, like UDP; never chosen) or reliable
    /// (every picture arrives, later on a slow line, like TCP)
    #[arg(long, value_name = "MODE", value_parser = parse_transport)]
    transport: Option<Delivery>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

/// `--decoder-library auto|ffmpeg|media_foundation` (plan 2026-11-17 P8).
pub(super) fn parse_library(text: &str) -> Result<DecoderLibrary, String> {
    DecoderLibrary::parse(text.trim()).ok_or_else(|| {
        format!("invalid decoder library: {text} (auto, ffmpeg or media_foundation)")
    })
}

/// `--display auto|direct3d11|wgpu` (P8).
pub(super) fn parse_display(text: &str) -> Result<DisplayChoice, String> {
    DisplayChoice::parse(text.trim())
        .ok_or_else(|| format!("invalid display: {text} (auto, direct3d11 or wgpu)"))
}

/// `--virtual-display auto|off|request` (plan 2026-11-17 P5).
pub(super) fn parse_virtual_display(text: &str) -> Result<VirtualDisplayWish, String> {
    VirtualDisplayWish::parse(text)
        .ok_or_else(|| format!("invalid virtual display: {text} (auto, off or request)"))
}

/// `--audio on|off` (plan 2026-11-17 P9).
pub(super) fn parse_audio(text: &str) -> Result<bool, String> {
    match text.trim() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("invalid audio: {text} (on or off)")),
    }
}

/// `--transport realtime|reliable` (user 2026-11-25).
pub(super) fn parse_transport(text: &str) -> Result<Delivery, String> {
    Delivery::parse(text).ok_or_else(|| format!("invalid transport: {text} (realtime or reliable)"))
}

/// `--display-size`: the viewer's window or a fixed size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DisplaySize {
    Window,
    Fixed(FrameSize),
}

/// `window`, or `WxH` with sides of 1 to 16384.
pub(super) fn parse_display_size(text: &str) -> Result<DisplaySize, String> {
    if text.trim().eq_ignore_ascii_case("window") {
        return Ok(DisplaySize::Window);
    }
    parse_size(text)
        .map(DisplaySize::Fixed)
        .ok_or_else(|| format!("invalid display size: {text} (WxH such as 2560x1440, or window)"))
}

/// `--codec auto|h264|hevc|av1|sharp|lossless` (BK6; `sharp` P9).
pub(super) fn parse_codec(text: &str) -> Result<CodecSetting, String> {
    CodecSetting::parse(text.trim())
        .ok_or_else(|| format!("invalid codec: {text} (auto, h264, hevc, av1, sharp or lossless)"))
}

/// `--decoder auto|hardware|software` (BK6).
pub(super) fn parse_decoder(text: &str) -> Result<DecoderPreference, String> {
    DecoderPreference::parse(text.trim())
        .ok_or_else(|| format!("invalid decoder: {text} (auto, hardware or software)"))
}

/// `--max-rate`: automatic or a ceiling in bit/s (plan 2026-11-14
/// „Bildkette“ B-09).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MaxRate {
    Auto,
    Bps(u64),
}

impl MaxRate {
    fn bps(self) -> Option<u64> {
        match self {
            Self::Auto => None,
            Self::Bps(bps) => Some(bps),
        }
    }
}

/// `auto`, or a positive number of bit/s with an optional decimal SI prefix
/// (k = 10³, M = 10⁶, G = 10⁹, SI brochure 9th edition, table 7). Rule „Wahl
/// gilt“ (2026-11-18): a ceiling below the lowest rate of the encoders
/// (`MIN_BITRATE_BPS`, 150 kbit/s) is refused instead of being raised
/// silently.
pub(super) fn parse_max_rate(text: &str) -> Result<MaxRate, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("auto") {
        return Ok(MaxRate::Auto);
    }
    let (number, factor) = match text.chars().last() {
        Some('k') | Some('K') => (&text[..text.len() - 1], 1e3),
        Some('M') => (&text[..text.len() - 1], 1e6),
        Some('G') => (&text[..text.len() - 1], 1e9),
        _ => (text, 1.0),
    };
    let value: f64 = number
        .trim()
        .parse()
        .map_err(|_| format!("invalid rate: {text} (auto, or bit/s such as 20M)"))?;
    let bps = (value * factor).round();
    if !bps.is_finite() || bps < 1.0 || bps > u64::MAX as f64 {
        return Err(format!("invalid rate: {text} (a positive number of bit/s)"));
    }
    let lowest = crate::screen::core::rate_control::MIN_BITRATE_BPS;
    if (bps as u64) < lowest {
        return Err(format!(
            "invalid rate: {text} (at least {} kbit/s, the lowest rate of the encoders)",
            lowest / 1000
        ));
    }
    Ok(MaxRate::Bps(bps as u64))
}

/// Rule „Wahl gilt“ (2026-11-18): `--decoder-library` and `--display` act
/// only in the Windows viewer; elsewhere they are refused instead of being
/// stored without effect. `windows`: the viewer runs on Windows.
pub(super) fn check_windows_only(
    library: Option<DecoderLibrary>,
    display: Option<DisplayChoice>,
    windows: bool,
) -> Result<(), String> {
    if windows {
        return Ok(());
    }
    let named: Vec<&str> = [
        library.map(|_| "--decoder-library"),
        display.map(|_| "--display"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if named.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{}: only the Windows viewer uses this choice; this system's viewer would ignore it",
            named.join(", ")
        ))
    }
}

#[derive(Args, Debug, PartialEq, Eq)]
pub(super) struct GridArgs {
    /// Only the members of this room (profile id, room id or name)
    #[arg(
        long,
        value_name = "RAUM",
        add = ArgValueCandidates::new(crate::cli::completions_access::room_candidates)
    )]
    room: Option<String>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

/// The target JSON of `--screen-view`: `relation`, `relation_id`,
/// `device_id`, `source` (text or null), `control`, and `fullscreen` only
/// when asked for (the start parser allows one value after the switch).
pub(super) fn target_json(
    choice: &DeviceChoice,
    source: Option<&str>,
    control: bool,
    fullscreen: bool,
) -> Result<Value, String> {
    if choice.device_id.is_empty() {
        return Err(format!(
            "{} has no device id yet; it appears after the first connection",
            choice.name
        ));
    }
    let source = source
        .map(|text| {
            SourceId::parse(text)
                .map(|source| source.to_string())
                .map_err(|error| format!("invalid source: {error}"))
        })
        .transpose()?;
    let relation_id = match &choice.target {
        crate::share::PeerOpenTarget::Direct { contact_id } => contact_id.clone(),
        crate::share::PeerOpenTarget::RoomDevice { room_id, .. } => room_id.clone(),
    };
    let mut target = json!({
        "relation": choice.relation,
        "relation_id": relation_id,
        "device_id": choice.device_id,
        "source": source,
        "control": control,
    });
    if fullscreen {
        target["fullscreen"] = Value::Bool(true);
    }
    Ok(target)
}

/// Starts the GUI program with `request`; the process ID.
fn launch(request: &StartRequest) -> Result<u32, String> {
    let gui = crate::desktop::os::launch::gui_binary()
        .map_err(|_| "Desktop-App nicht installiert".to_string())?;
    let args = crate::desktop::os::open_target::window_args(request);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let child = crate::desktop::os::platform::process::spawn_detached(&gui, &args)
        .map_err(|error| format!("{}: {error}", gui.display()))?;
    let pid = child.id();
    crate::desktop::os::launch::reap(child);
    Ok(pid)
}

fn report(json: bool, pid: u32) {
    super::output::print(json, &json!({"launched": true, "pid": pid}), || {
        format!("launched\t{pid}\n")
    });
}

/// Open an exact generation-bearing selector; the host rechecks its lifetime
/// and the viewer's rights when the native window connects.
pub(crate) fn open_family(device: &str, source: &str, control: bool) -> Result<u32, String> {
    let parsed = SourceId::parse(source).map_err(|error| error.to_string())?;
    if crate::screen::core::family_protocol::selector(&parsed).is_none() {
        return Err("source must be the exact window:@family selector from screen families".into());
    }
    let choice = super::select_device(device)?;
    let target = target_json(&choice, Some(source), control, false)?;
    launch(&StartRequest::ScreenView {
        target_json: target.to_string(),
    })
}

pub(super) fn run_view(args: ViewArgs) -> Result<i32, String> {
    check_windows_only(args.decoder_library, args.display, cfg!(windows))?;
    let choice = super::select_device(&args.device)?;
    // The whole call is checked before anything is stored (Codex 10/14: an
    // invalid `--source` failed after its settings were saved).
    let mut target = target_json(
        &choice,
        args.source.as_deref(),
        args.control,
        args.fullscreen,
    )?;
    served_check(&args, &choice)?;
    if let Some(rate) = args.max_rate {
        crate::app::screen_prefs::set_max_rate_bps(rate.bps())
            .map_err(|error| format!("cannot store the maximum rate: {error}"))?;
    }
    if args.codec.is_some() || args.decoder.is_some() {
        crate::app::screen_prefs::set_codec_choice(args.codec, args.decoder)
            .map_err(|error| format!("cannot store the codec choice: {error}"))?;
    }
    if args.decoder_library.is_some() || args.display.is_some() {
        crate::app::screen_prefs::set_output_choice(args.decoder_library, args.display)
            .map_err(|error| format!("cannot store the decoder library or display: {error}"))?;
    }
    if args.virtual_display.is_some() || args.display_size.is_some() {
        let size = args.display_size.map(|size| match size {
            DisplaySize::Window => None,
            DisplaySize::Fixed(size) => Some(size),
        });
        crate::app::screen_prefs::set_virtual_display_choice(args.virtual_display, size)
            .map_err(|error| format!("cannot store the display choice: {error}"))?;
    }
    if let Some(on) = args.audio {
        crate::app::screen_audio::set_audio(on)
            .map_err(|error| format!("cannot store the audio choice: {error}"))?;
    }
    if let Some(mode) = args.transport {
        crate::app::screen_delivery::set_delivery(mode)
            .map_err(|error| format!("cannot store the transport choice: {error}"))?;
    }
    // This call's settings go to its own window too: a second call that
    // stores others before the window reads the file does not decide for it.
    target["choices"] = crate::app::screen_prefs::start_choices();
    let request = StartRequest::ScreenView {
        target_json: target.to_string(),
    };
    report(args.json, launch(&request)?);
    Ok(0)
}

/// The device's encoders from its probe; `None` when it does not answer or
/// shows nothing to this device (the window then decides per session).
pub(super) fn probed_encoders(
    target: &crate::share::PeerOpenTarget,
) -> Option<Vec<crate::screen::core::negotiate::CodecOffer>> {
    use crate::share::access_grant::CapabilityDecision;
    match crate::daemon::screen_probe(target.clone()) {
        Ok(report) if report.supported && report.view != CapabilityDecision::Off => {
            Some(report.codecs)
        }
        _ => None,
    }
}

/// `codec`'s reason against the device's `host` encoders and this device's
/// decoders under `decoder` and `library`, and the codecs that run (keys
/// and labels); `None` when `codec` runs.
pub(super) fn served_reason(
    codec: CodecSetting,
    host: &[crate::screen::core::negotiate::CodecOffer],
    decoder: DecoderPreference,
    library: DecoderLibrary,
) -> Option<(String, Vec<CodecSetting>)> {
    use crate::app::screen_prefs::codec_offer::{codec_verdict, Viewer};
    let offers = |library| {
        crate::screen::core::decoder_library::set_decoder_library(library);
        crate::screen::core::negotiate::set_decoder_preference(decoder);
        crate::screen::os::codec_inventory::registry().decoder_offers()
    };
    // A chosen library without decoders here runs as „Automatisch“ in the
    // window (`codec_offer::effective_library`), so it is checked as that.
    let mut library = library;
    let mut decoders = offers(library);
    if !decoders.iter().any(|offer| offer.codec.is_video()) {
        library = DecoderLibrary::Automatic;
        decoders = offers(library);
    }
    let viewer = Viewer {
        decoders: &decoders,
        decoder,
        library,
    };
    let reason = codec_verdict(codec, Some(host), &viewer)
        .reason()?
        .to_string();
    let served = CodecSetting::ALL
        .into_iter()
        .filter(|other| {
            codec_verdict(*other, Some(host), &viewer)
                .reason()
                .is_none()
        })
        .collect();
    Some((reason, served))
}

/// A named `--codec` or `--decoder` the device cannot serve with this one:
/// the German reason and the codecs it serves (`codec_offer`).
fn served_check(args: &ViewArgs, choice: &DeviceChoice) -> Result<(), String> {
    if args.codec.is_none() && args.decoder.is_none() {
        return Ok(());
    }
    let Some(host) = probed_encoders(&choice.target) else {
        return Ok(());
    };
    let codec = args
        .codec
        .unwrap_or_else(crate::app::screen_prefs::codec_setting);
    let decoder = args
        .decoder
        .unwrap_or_else(crate::app::screen_prefs::decoder_choice);
    let library = args
        .decoder_library
        .unwrap_or_else(|| crate::app::screen_prefs::stored_output().0);
    let Some((reason, served)) = served_reason(codec, &host, decoder, library) else {
        return Ok(());
    };
    let served: Vec<String> = served
        .iter()
        .map(|codec| format!("{} ({})", codec.key(), codec.label()))
        .collect();
    Err(format!(
        "Codec „{}“ mit Dekodierer „{}“: {reason}\nMit {} möglich: {}",
        codec.label(),
        decoder.label(),
        choice.name,
        served.join(", ")
    ))
}

/// The local profile ID of `room` (profile ID, room ID or name).
fn room_profile_id(room: &str) -> Result<String, String> {
    let Ok(snapshot) = crate::daemon::share_worker_snapshot() else {
        // The window reads the profiles itself and says what it misses.
        return Ok(room.to_string());
    };
    snapshot
        .profiles
        .rooms
        .iter()
        .find(|candidate| {
            candidate.id == room || candidate.room_id == room || candidate.name == room
        })
        .map(|found| found.id.clone())
        .ok_or_else(|| format!("room not found: {room}"))
}

pub(super) fn run_grid(args: GridArgs) -> Result<i32, String> {
    let room = args.room.as_deref().map(room_profile_id).transpose()?;
    let request = StartRequest::ScreenGrid { room };
    report(args.json, launch(&request)?);
    Ok(0)
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
