//! `se screen bench (DEVICE | --local)` (C9): the test picture over a
//! session with another device, measured by the background worker, or
//! encoder and decoder of this device alone. The other device's user may be
//! asked first („Fragen“); the command then waits without a time limit and
//! says so on stderr.
use clap::Args;
use clap_complete::ArgValueCandidates;

use crate::screen::core::bench::{
    run_local, BenchOptions, DEFAULT_FPS, DEFAULT_SECONDS, DEFAULT_SIZE,
};
use crate::screen::core::negotiate::CodecPreference;
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
    #[arg(long, value_parser = ["auto", "h264", "tiles"], default_value = "auto")]
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
        options.decoders()?;
        Ok(options)
    }
}

pub(super) fn run(args: BenchArgs) -> Result<i32, String> {
    let options = args.options()?;
    let report = match args.device.as_deref() {
        None => run_local(options).map_err(|error| error.to_string())?,
        Some(device) => {
            let choice = super::select_device(device)?;
            let name = choice.name.clone();
            let pending = |capability: Capability| {
                let what = match capability {
                    Capability::ScreenControl => "steuern",
                    _ => "ansehen",
                };
                eprintln!("Warte auf Bestätigung an {name}: Bildschirm {what} …");
            };
            crate::daemon::screen_bench(choice.target, &options, pending)?
        }
    };
    super::output::print(args.json, &report, || super::output::bench_text(&report));
    Ok(0)
}
