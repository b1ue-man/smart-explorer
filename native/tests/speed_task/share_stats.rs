//! The QUIC statistics of the Share cases (diagnosis of E13): each Share
//! device of the stage writes one JSON line per finished transfer burst into
//! `share-quic-stats.jsonl` of its profile (`share/core/quic_stats_hub.rs`,
//! development builds only). A case marks the end of both files when it
//! connects and, when it finishes, reads what the two devices wrote since: the
//! lines go into the report line of the case (`share_quic`, with a short text
//! per line in `share_quic_summary`) and onto stdout as `SHARE_QUIC <case>
//! <json>`.
//!
//! The file of the device the test program runs in (`main`) lies in its data
//! folder (`XDG_DATA_HOME`, `APPDATA` on Windows); the file of the other
//! device (`remote`) lies next to the export folder the stage created
//! (`<stage folder>/share-remote/data/smart_explorer`, the layout of
//! `sync-reliability-share-fixture.py`) unless `SE_SPEED_SHARE_REMOTE_STATS`
//! names it.
#![allow(dead_code)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const FILE: &str = "share-quic-stats.jsonl";
const KIND: &str = "share_quic_burst";
/// The reporter writes a burst after two quiet half seconds; a case waits at
/// most this long for the lines of both devices.
const MAX_WAIT: Duration = Duration::from_secs(6);
/// No new line for this long: the devices have written everything.
const QUIET: Duration = Duration::from_millis(1500);
/// Without any line after this long no device writes statistics (a release
/// build, or `SE_TEST_NO_SHARE_STATS`): the case does not wait any longer.
const FIRST_LINE_WAIT: Duration = Duration::from_secs(3);

struct Cursor {
    device: &'static str,
    path: PathBuf,
    offset: u64,
}

static CURSORS: Mutex<Vec<Cursor>> = Mutex::new(Vec::new());

/// What a case read: the lines and one short text per line.
#[derive(Default)]
pub struct Collected {
    pub lines: Vec<Value>,
    pub summaries: Vec<String>,
}

fn data_home() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(PathBuf::from);
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
        })
}

/// The statistics files of the devices of the stage; none when the stage has
/// no Share pair.
fn candidate_files() -> Vec<(&'static str, PathBuf)> {
    let Some(export) = std::env::var_os("SE_SPEED_SHARE_DIR").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    if let Some(home) = data_home() {
        files.push(("main", home.join("smart_explorer").join(FILE)));
    }
    let remote = std::env::var_os("SE_SPEED_SHARE_REMOTE_STATS")
        .map(PathBuf::from)
        .or_else(|| {
            export.parent().map(|stage| {
                stage
                    .join("share-remote")
                    .join("data")
                    .join("smart_explorer")
                    .join(FILE)
            })
        });
    if let Some(path) = remote {
        files.push(("remote", path));
    }
    files
}

fn length(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |metadata| metadata.len())
}

/// Starts a case: whatever the devices wrote before is not its.
pub fn mark() {
    let cursors = candidate_files()
        .into_iter()
        .map(|(device, path)| Cursor {
            device,
            offset: length(&path),
            path,
        })
        .collect();
    if let Ok(mut slot) = CURSORS.lock() {
        *slot = cursors;
    }
}

/// The complete lines of `text` after byte `from` that are bursts, and the
/// offset behind the last complete line. A partial last line (the device is
/// still writing it), a line that is not JSON and a line of another kind are
/// skipped; an offset behind the end (the file was replaced) starts at zero.
pub fn lines_after(text: &[u8], from: u64) -> (Vec<Value>, u64) {
    let start = usize::try_from(from)
        .ok()
        .filter(|start| *start <= text.len())
        .unwrap_or(0);
    let mut values = Vec::new();
    let mut offset = start;
    let mut rest = &text[start..];
    while let Some(end) = rest.iter().position(|byte| *byte == b'\n') {
        let (line, tail) = rest.split_at(end + 1);
        offset += line.len();
        rest = tail;
        if let Ok(value) = serde_json::from_slice::<Value>(&line[..end]) {
            if value["kind"] == KIND {
                values.push(value);
            }
        }
    }
    (values, offset as u64)
}

fn number(value: &Value, path: &[&str]) -> f64 {
    path.iter()
        .try_fold(value, |value, key| value.get(*key))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

/// One line of text per burst, the values that tell where a transfer lost its
/// speed: rate, round trip, window, loss, time per phase, CPU.
pub fn summary(device: &str, line: &Value) -> String {
    let burst = |path: &[&str]| number(line, &[&["burst"][..], path].concat());
    format!(
        "QUIC {device}/{side} #{seq} {cc}: {dur:.1} s, tx {tx:.1} MB/s, rx {rx:.1} MB/s, rtt {rtt:.1} ms \
         ({rtt_min:.1}..{rtt_max:.1}), cwnd {cwnd:.0} KiB ({cwnd_min:.0}..{cwnd_max:.0}), \
         lost {lost:.0} pkts / {lost_bytes:.0} B, congestion {events:.0}, mtu {mtu:.0}, \
         streams {streams:.0}, lz4 {lz4:.0} ms, stage write {write:.0} ms, net wait {net:.0} ms, \
         process cpu {cpu:.0} ms, rcvbuf errors {rcvbuf:.0} (rmem_max {rmem:.0})",
        side = line["side"].as_str().unwrap_or("?"),
        seq = number(line, &["seq"]),
        cc = line["cc"].as_str().unwrap_or("?"),
        dur = burst(&["duration_ms"]) / 1000.0,
        tx = burst(&["udp_tx_mb_per_s"]),
        rx = burst(&["udp_rx_mb_per_s"]),
        rtt = burst(&["rtt_ms"]),
        rtt_min = burst(&["rtt_ms_min"]),
        rtt_max = burst(&["rtt_ms_max"]),
        cwnd = burst(&["cwnd_bytes"]) / 1024.0,
        cwnd_min = burst(&["cwnd_bytes_min"]) / 1024.0,
        cwnd_max = burst(&["cwnd_bytes_max"]) / 1024.0,
        lost = burst(&["lost_packets"]),
        lost_bytes = burst(&["lost_bytes"]),
        events = burst(&["congestion_events"]),
        mtu = burst(&["mtu"]),
        streams = burst(&["streams"]),
        lz4 = burst(&["meter", "lz4_ms"]),
        write = burst(&["meter", "stage_write_ms"]),
        net = burst(&["meter", "net_wait_ms"]),
        cpu = burst(&["process_cpu_ms"]),
        rcvbuf = burst(&["kernel_udp", "rcvbuf_errors"]),
        rmem = number(line, &["socket_buffers", "rmem_max"]),
    )
}

/// Reads what both devices wrote since the mark, waiting until both have
/// written their lines of the finished transfer and nothing more arrives.
pub fn collect() -> Collected {
    let Ok(mut cursors) = CURSORS.lock() else {
        return Collected::default();
    };
    if cursors.is_empty() {
        return Collected::default();
    }
    let started = Instant::now();
    let mut quiet_since = Instant::now();
    let mut seen = 0usize;
    let mut found: Vec<(&'static str, Vec<Value>)>;
    loop {
        found = Vec::new();
        let mut total = 0usize;
        for cursor in cursors.iter() {
            let text = std::fs::read(&cursor.path).unwrap_or_default();
            let (lines, _) = lines_after(&text, cursor.offset);
            total += lines.len();
            found.push((cursor.device, lines));
        }
        if total != seen {
            seen = total;
            quiet_since = Instant::now();
        }
        let both = found.iter().all(|(_, lines)| !lines.is_empty());
        let nothing = total == 0 && started.elapsed() >= FIRST_LINE_WAIT;
        if (both && quiet_since.elapsed() >= QUIET) || nothing || started.elapsed() >= MAX_WAIT {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    for cursor in cursors.iter_mut() {
        let text = std::fs::read(&cursor.path).unwrap_or_default();
        cursor.offset = lines_after(&text, cursor.offset).1;
    }
    let mut collected = Collected::default();
    for (device, lines) in found {
        for mut line in lines {
            collected.summaries.push(summary(device, &line));
            line["device"] = Value::String(device.to_string());
            collected.lines.push(line);
        }
    }
    collected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burst_line(seq: u64) -> String {
        format!(
            r#"{{"kind":"share_quic_burst","seq":{seq},"side":"host","cc":"cubic","socket_buffers":{{"rmem_max":212992}},"burst":{{"duration_ms":30000,"udp_tx_mb_per_s":58.5,"rtt_ms":2.5,"cwnd_bytes":2048000,"lost_packets":12,"meter":{{"net_wait_ms":900.4}},"kernel_udp":{{"rcvbuf_errors":4}}}}}}"#
        )
    }

    #[test]
    fn share_stats_reader_takes_complete_burst_lines_after_the_mark() {
        let first = format!("{}\n", burst_line(1));
        let second = format!("{}\n", burst_line(2));
        let mut text = format!("{first}{second}").into_bytes();
        // A line of another kind, a damaged line and a partial last line.
        text.extend_from_slice(b"{\"kind\":\"other\"}\nnot json\n");
        text.extend_from_slice(burst_line(3).as_bytes());

        let (all, offset) = lines_after(&text, 0);
        assert_eq!(all.len(), 2, "the partial third line is not read yet");
        assert_eq!(offset as usize, text.len() - burst_line(3).len());
        let (after_first, _) = lines_after(&text, first.len() as u64);
        assert_eq!(after_first.len(), 1);
        assert_eq!(after_first[0]["seq"], 2);

        // The third line arrives completely: it is read from the old offset.
        text.push(b'\n');
        let (rest, end) = lines_after(&text, offset);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0]["seq"], 3);
        assert_eq!(end as usize, text.len());
        // A file that was replaced by a shorter one is read from its start.
        let (again, _) = lines_after(&text[..first.len()], 10_000);
        assert_eq!(again.len(), 1);
    }

    #[test]
    fn share_stats_summary_names_rate_window_loss_and_waiting() {
        let line: Value = serde_json::from_str(&burst_line(7)).expect("a line");
        let text = summary("remote", &line);
        for expected in [
            "QUIC remote/host #7 cubic",
            "30.0 s",
            "tx 58.5 MB/s",
            "rtt 2.5 ms",
            "cwnd 2000 KiB",
            "lost 12 pkts",
            "net wait 900 ms",
            "rcvbuf errors 4 (rmem_max 212992)",
        ] {
            assert!(text.contains(expected), "{expected:?} missing in {text}");
        }
        // Missing fields read as zero, never as a panic.
        let bare: Value = serde_json::from_str(r#"{"kind":"share_quic_burst"}"#).expect("json");
        assert!(summary("main", &bare).contains("QUIC main/? #0"));
    }
}
