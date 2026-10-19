//! `se screen snapshot DEVICE [--source QUELLE] --out DATEI [--max-size BxH]
//! [--force] [--json]` (SCR-M4): one lossless still picture of another
//! device's screen as a PNG file. The source is `monitor:<id>` or
//! `window:<id>` (`se screen probe` and `se screen windows` list them), the
//! primary monitor without `--source`. The picture has the size of the
//! source, at most `--max-size`. The file is written beside its target under
//! a temporary name and renamed, so a failed call leaves no half file; an
//! existing file is only replaced with `--force`. The device's user may be
//! asked first („Fragen“); the command then waits without a time limit and
//! says so on stderr. JSON: `{"path","width","height","bytes","source"}`;
//! text: `snapshot`, path, `WxH`, bytes and source, tab-separated.
use std::path::{Path, PathBuf};

use clap::Args;
use clap_complete::ArgValueCandidates;
use serde_json::{json, Value};

use super::bench::{parse_size, SizeArg};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::snapshot::encode_png;
use crate::screen::core::window_list::SourceId;

#[derive(Args, Debug, PartialEq, Eq)]
pub(super) struct SnapshotArgs {
    #[arg(
        help = "Device: share:// endpoint, name, id or fingerprint prefix",
        add = ArgValueCandidates::new(crate::cli::completions_screen::device_candidates)
    )]
    device: String,
    /// Source to show: monitor:<id> or window:<id> (default: the primary monitor)
    #[arg(long, value_name = "QUELLE")]
    source: Option<String>,
    /// PNG file to write
    #[arg(long, value_name = "DATEI")]
    out: PathBuf,
    /// Largest size of the picture
    #[arg(long, value_name = "BxH", value_parser = parse_size)]
    max_size: Option<SizeArg>,
    /// Replace an existing file
    #[arg(long)]
    force: bool,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

/// The refusal of an existing file, before the other device is asked.
pub(super) fn check_target(out: &Path, force: bool) -> Result<(), String> {
    if !force && std::fs::symlink_metadata(out).is_ok() {
        return Err(format!(
            "{} existiert schon; --force überschreibt",
            out.display()
        ));
    }
    Ok(())
}

/// Writes `png` to `out` through a temporary file in the same folder.
pub(super) fn write_png(out: &Path, png: &[u8], force: bool) -> Result<(), String> {
    check_target(out, force)?;
    let mut temporary = out.as_os_str().to_owned();
    temporary.push(format!(".se-tmp-{}", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let written = std::fs::write(&temporary, png)
        .and_then(|()| std::fs::rename(&temporary, out))
        .map_err(|error| format!("{}: {error}", out.display()));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// The JSON of a saved still picture.
pub(super) fn snapshot_json(path: &Path, size: FrameSize, bytes: usize, source: &str) -> Value {
    json!({
        "path": path.display().to_string(),
        "width": size.width,
        "height": size.height,
        "bytes": bytes,
        "source": source,
    })
}

pub(super) fn snapshot_text(path: &Path, size: FrameSize, bytes: usize, source: &str) -> String {
    format!(
        "snapshot\t{}\t{}x{}\t{bytes}\t{source}\n",
        path.display(),
        size.width,
        size.height
    )
}

pub(super) fn run(args: SnapshotArgs) -> Result<i32, String> {
    check_target(&args.out, args.force)?;
    let source = args
        .source
        .as_deref()
        .map(SourceId::parse)
        .transpose()
        .map_err(|error| format!("--source: {error}"))?;
    let choice = super::select_device(&args.device)?;
    let still = crate::daemon::screen_snapshot(
        choice.target,
        source,
        args.max_size.map(|size| size.0),
        super::pending_notice(choice.name),
    )?;
    let png = encode_png(&still.image).map_err(|error| error.to_string())?;
    write_png(&args.out, &png, args.force)?;
    let path = std::path::absolute(&args.out).unwrap_or_else(|_| args.out.clone());
    let size = still.image.size();
    let value = snapshot_json(&path, size, png.len(), &still.source);
    super::output::print(args.json, &value, || {
        snapshot_text(&path, size, png.len(), &still.source)
    });
    Ok(0)
}
