// Quick perf bench using the same scanner module as the GUI.
// Usage: cargo run --release --bin bench -- <path>
use crossbeam_channel::unbounded;
use std::env;
use std::path::PathBuf;
use std::time::Instant;

#[path = "../scanner/mod.rs"]
mod scanner;
#[path = "../types/mod.rs"]
mod types;

use scanner::ScanMessage;

fn main() {
    let mut args = env::args().skip(1);
    let mut target: Option<PathBuf> = None;
    let mut depth: Option<u32> = None;
    while let Some(a) = args.next() {
        if let Some(d) = a.strip_prefix("--depth=") {
            depth = d.parse().ok();
        } else {
            target = Some(PathBuf::from(a));
        }
    }
    let target = target.unwrap_or_else(|| PathBuf::from("."));
    println!("Scanning: {} (depth={:?})", target.display(), depth);

    let (tx, rx) = unbounded();
    let t0 = Instant::now();
    let _handle = scanner::start_scan(target.clone(), false, depth, tx);

    let mut entry_count: u64 = 0;
    let mut bytes: u64 = 0;
    let mut errors: u64 = 0;
    let mut elapsed: u64 = 0;
    let mut first_entry_at: Option<u64> = None;

    while let Ok(msg) = rx.recv() {
        match msg {
            ScanMessage::Entries(c) => {
                if first_entry_at.is_none() {
                    first_entry_at = Some(t0.elapsed().as_millis() as u64);
                }
                entry_count += c.len() as u64;
            }
            ScanMessage::Progress(_) => {}
            ScanMessage::Error(e) => eprintln!("error: {}", e),
            ScanMessage::FailedPaths(paths) => {
                eprintln!("  ({} failed paths reported)", paths.len());
                for (p, m) in paths.iter().take(5) {
                    eprintln!("    {}: {}", p, m);
                }
            }
            ScanMessage::Done(p) => {
                bytes = p.bytes;
                errors = p.errors;
                elapsed = p.elapsed_ms;
                break;
            }
        }
    }

    let total_elapsed_ms = t0.elapsed().as_millis() as u64;
    let rate = if elapsed > 0 {
        (entry_count as f64 / elapsed as f64) * 1000.0
    } else {
        0.0
    };
    println!("---");
    println!("Entries:           {}", entry_count);
    println!(
        "Bytes:             {:.2} GB",
        bytes as f64 / 1024.0 / 1024.0 / 1024.0
    );
    println!("Errors:            {}", errors);
    println!("Time (worker):     {} ms", elapsed);
    println!("Time (incl. drain):{} ms", total_elapsed_ms);
    println!("First entry at:    {:?} ms", first_entry_at);
    println!("Rate:              {:.0} entries/s", rate);
}
