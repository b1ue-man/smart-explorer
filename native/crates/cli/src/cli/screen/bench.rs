//! `se screen bench (DEVICE | --local)` (C9): the test picture over a
//! session with another device, measured by the background worker, or
//! encoder and decoder of this device alone. The other device's user may be
//! asked first („Fragen“); the command then waits without a time limit and
//! says so on stderr. The report carries the stations of the picture path
//! the measuring side can time (`stages_ms`, each with `p50` and `p95` in ms:
//! `capture`, `encode` and `decode` with `--local`; `decode` and
//! `capture_to_arrival`, the host's capture until the last record arrived,
//! for a session with another device); `--json` prints them with the report
//! and the text shows one `stage_ms` line per station.
//!
//! `--source QUELLE` (plan 2026-11-14 „Bildkette“ BK0): the host captures
//! `monitor:<id>` or `window:<id>` (`se screen probe` and `se screen windows`
//! list them) instead of the test picture. A host that announces
//! `frame_timing_v1` adds its stations to `stages_ms` (`capture`, `encode`,
//! `worker_handoff`, `send`, `network`, `tick_jitter`, `input`,
//! `input_inject`); `host_timing` says `reported` or `unsupported_by_host`.
//! A monitor the host does not have ends with the code `source_not_found`.
//!
//! User rule 2026-11-24: a `--codec` the device and this one cannot run
//! together is refused before the measurement with the German reason and
//! the codecs they can run (`view::served_reason`; exit code non-zero).
use clap::Args;
use clap_complete::ArgValueCandidates;

use crate::screen::core::bench::{
    run_local, BenchOptions, DEFAULT_FPS, DEFAULT_SECONDS, DEFAULT_SIZE,
};
use crate::screen::core::negotiate::{CodecPreference, CodecSetting};
use crate::screen::core::pixel_convert::FrameSize;
use crate::share::access_grant::Capability;

#[derive(Args, Debug, PartialEq, Eq)]
pub(super) struct BenchArgs {
    #[arg(
        required_unless_present = "local",
        conflicts_with = "local",
        help = "Device: share:// endpoint, name, id or fingerprint prefix",
        add = ArgValueCandidates::new(crate::cli::completions_screen::device_candidates)
    )]
    device: Option<String>,
    /// Encoder and decoder of this device only, no network
    #[arg(long)]
    local: bool,
    /// Codec of the measurement
    #[arg(long, value_parser = ["auto", "h264", "hevc", "av1", "tiles"], default_value = "auto")]
    codec: String,
    /// Size of the test picture
    #[arg(long, value_name = "WxH", value_parser = parse_size, default_value_t = DEFAULT_SIZE.into())]
    size: SizeArg,
    /// Frames per second
    #[arg(long, default_value_t = DEFAULT_FPS)]
    fps: u16,
    /// Length of the measurement
    #[arg(long, default_value_t = DEFAULT_SECONDS)]
    seconds: u32,
    /// The host's real capture instead of the test picture: monitor:<id> or window:<id>
    #[arg(long, value_name = "QUELLE", conflicts_with = "local")]
    source: Option<String>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

/// `--size WxH`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SizeArg(pub(super) FrameSize);

impl From<FrameSize> for SizeArg {
    fn from(size: FrameSize) -> Self {
        Self(size)
    }
}

impl std::fmt::Display for SizeArg {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}x{}", self.0.width, self.0.height)
    }
}

pub(super) fn parse_size(text: &str) -> Result<SizeArg, String> {
    let (width, height) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("size {text:?} is not WIDTHxHEIGHT"))?;
    let side = |value: &str| {
        value
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("size {text:?} is not WIDTHxHEIGHT"))
    };
    Ok(SizeArg(FrameSize::new(side(width)?, side(height)?)))
}

impl BenchArgs {
    fn options(&self) -> Result<BenchOptions, String> {
        let codec = CodecPreference::parse(&self.codec)
            .ok_or_else(|| format!("unknown codec {:?}", self.codec))?;
        let options = BenchOptions {
            codec,
            size: self.size.0,
            fps: self.fps,
            seconds: self.seconds,
        };
        options.validate()?;
        options.decoders(&crate::screen::os::codec_inventory::registry())?;
        Ok(options)
    }

    /// `--source` as the host reads it; the test picture is no source.
    pub(super) fn source(&self) -> Result<Option<String>, String> {
        let Some(text) = self.source.as_deref() else {
            return Ok(None);
        };
        match crate::screen::core::window_list::SourceId::parse(text) {
            Ok(crate::screen::core::window_list::SourceId::Synthetic) => {
                Err("--source names the host's screen; omit it for the test picture".into())
            }
            Ok(source) => Ok(Some(source.to_string())),
            Err(error) => Err(format!("invalid source: {error}")),
        }
    }
}

pub(super) fn run(args: BenchArgs) -> Result<i32, String> {
    let options = args.options()?;
    let source = args.source()?;
    let report = match args.device.as_deref() {
        None => run_local(options, &crate::screen::os::codec_inventory::registry())
            .map_err(|error| error.to_string())?,
        Some(device) => {
            let choice = super::select_device(device)?;
            let name = choice.name.clone();
            served_check(&args.codec, &choice)?;
            let pending = |capability: Capability| {
                let what = match capability {
                    Capability::ScreenControl => "steuern",
                    _ => "ansehen",
                };
                eprintln!("Warte auf Bestätigung an {name}: Bildschirm {what} …");
            };
            crate::daemon::screen_bench(choice.target, &options, source, pending)?
        }
    };
    super::output::print(args.json, &report, || super::output::bench_text(&report));
    // P3: the counters are printed; no intact picture stays an error.
    match report.failure {
        Some(failure) => Err(failure),
        None => Ok(0),
    }
}

/// The bench's codec as the viewer's „Codec“ (`tiles` is „Verlustfrei“).
fn setting_of(codec: CodecPreference) -> CodecSetting {
    match codec {
        CodecPreference::Auto => CodecSetting::Auto,
        CodecPreference::H264 => CodecSetting::H264,
        CodecPreference::Hevc => CodecSetting::Hevc,
        CodecPreference::Av1 => CodecSetting::Av1,
        CodecPreference::Sharp => CodecSetting::Sharp,
        CodecPreference::Tiles => CodecSetting::Lossless,
    }
}

/// The bench's key of a viewer's „Codec“; `None` for one it does not take.
fn bench_key(setting: CodecSetting) -> Option<&'static str> {
    match setting {
        CodecSetting::Sharp => None,
        CodecSetting::Lossless => Some("tiles"),
        other => Some(other.key()),
    }
}

/// A named `--codec` the device cannot serve with this one: the German
/// reason and the codecs it serves. The bench offers every decoder stage
/// and no library choice (`BenchOptions::decoders`).
fn served_check(codec: &str, choice: &super::DeviceChoice) -> Result<(), String> {
    let Some(preference) = CodecPreference::parse(codec) else {
        return Ok(());
    };
    let setting = setting_of(preference);
    if matches!(setting, CodecSetting::Auto | CodecSetting::Lossless) {
        return Ok(());
    }
    let Some(host) = super::view::probed_encoders(&choice.target) else {
        return Ok(());
    };
    use crate::screen::core::decoder_library::DecoderLibrary;
    use crate::screen::core::negotiate::DecoderPreference;
    let decoder = DecoderPreference::Automatic;
    let Some((reason, served)) =
        super::view::served_reason(setting, &host, decoder, DecoderLibrary::Automatic)
    else {
        return Ok(());
    };
    let served: Vec<&str> = served.into_iter().filter_map(bench_key).collect();
    Err(format!(
        "Codec „{}“: {reason}\nMit {} möglich: {}",
        setting.label(),
        choice.name,
        served.join(", ")
    ))
}

#[cfg(test)]
#[path = "bildkette_bench_tests.rs"]
mod bildkette_tests;
