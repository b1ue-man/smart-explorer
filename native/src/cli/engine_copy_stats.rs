//! The `--stats` line of `se cp`/`get`/`put`: bytes, time, rate and the
//! resource that limited the copy (U49-7).
use crate::cli::tree_observer::{TreeObserver, TreeProgress};
use crate::format::format_bytes;
use std::io::{self, Write};
#[path = "optical_status.rs"]
mod optical_status;

/// Bytes per second over the whole run; 0 without a measured time.
pub(in crate::cli) fn average_rate(bytes: u64, elapsed_ms: u64) -> u64 {
    if elapsed_ms == 0 {
        return 0;
    }
    (u128::from(bytes) * 1000 / u128::from(elapsed_ms)).min(u128::from(u64::MAX)) as u64
}

/// The words of `--stats` for a `Limiter::code()`.
pub(in crate::cli) fn limiter_stats_text(code: &str) -> &'static str {
    match code {
        "link" => "link",
        "remote" => "server or link",
        "source" => "source disk",
        "target" => "target disk",
        "se_cpu" => "se cpu",
        "se_thread" => "se threads",
        "memory" => "memory",
        _ => "none",
    }
}

/// The `--stats` line. `wall_ms` is the time the command measured itself; the
/// engine's own time is used when it ran there.
pub(in crate::cli) fn stats_line(progress: &TreeProgress, wall_ms: u64) -> String {
    let (bytes, elapsed_ms) = if progress.engine {
        let elapsed = if progress.elapsed_ms > 0 {
            progress.elapsed_ms
        } else {
            wall_ms
        };
        (progress.bytes_written, elapsed)
    } else {
        (progress.bytes_written.max(progress.bytes_read), wall_ms)
    };
    let seconds = format!("{:.1}", elapsed_ms as f64 / 1000.0);
    let rate = format_bytes(average_rate(bytes, elapsed_ms));
    let mut line = if progress.engine {
        format!(
            "transferred {} files, {} in {seconds} s ({rate}/s); limited by: {}",
            progress.files_written,
            format_bytes(bytes),
            limiter_stats_text(&progress.limiter)
        )
    } else {
        format!(
            "transferred {} in {seconds} s ({rate}/s); limited by: unknown (legacy path)",
            format_bytes(bytes)
        )
    };
    if let Some(optical) = optical_status::text(&progress.optical_readers) { line.push_str("; "); line.push_str(&optical); }
    line
}

/// Writes the `--stats` line to stderr.
pub(in crate::cli) fn print_stats(observer: &TreeObserver, wall_ms: u64) {
    let line = stats_line(&observer.progress(), wall_ms);
    let _ = writeln!(io::stderr().lock(), "{line}");
}
