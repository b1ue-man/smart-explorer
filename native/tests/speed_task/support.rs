//! Support of the speed cases (U55-4): environment, the report line of every
//! case (M-1), capacities written by the reference cases, the share check and
//! the one retry of a shortfall. The engine run is `run.rs`, programs are
//! `tools.rs`, test data `data.rs`, servers `remote.rs`.
//!
//! Contract with `native/rdt_task/speed*.py` (U55-3): a case reads the
//! `SE_SPEED_*` variables it needs. A missing variable or tool makes it print
//! `SPEED_NOT_RUN <name> <reason>` and end without a panic (the stage records
//! "nicht ausgeführt"); every measured case appends one JSON line to
//! `SE_SPEED_REPORT` and checks its requirement with a panic whose text is
//! `<name>: <value> < <required> × <capacity> (<capacity name> aus <source>)`.
#![allow(dead_code)]

#[path = "../transfer_task/support.rs"]
pub mod base;

pub use base::{job, local_folder, remote_folder, slash, unique};

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MIB: u64 = 1024 * 1024;
/// Largest wait of one case (M-4: ten minutes, then the case has failed).
pub const CASE_LIMIT: Duration = Duration::from_secs(600);

/// One measured case: its name and the coordinates of the report line.
#[derive(Clone, Copy)]
pub struct Case {
    pub name: &'static str,
    pub way: &'static str,
    pub direction: &'static str,
    pub profile: &'static str,
}

impl Case {
    pub const fn new(
        name: &'static str,
        way: &'static str,
        direction: &'static str,
        profile: &'static str,
    ) -> Self {
        Case {
            name,
            way,
            direction,
            profile,
        }
    }
}

// ----------------------------------------------------------------- environment

/// A non-empty environment variable.
pub fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// A variable the stage must have set, or the reason the case cannot run.
pub fn need(name: &str, what: &str) -> Result<String, String> {
    var(name).ok_or_else(|| format!("{name} fehlt ({what})"))
}

/// The fast volume of the stage (`/mnt/…`).
pub fn fast_dir() -> Result<PathBuf, String> {
    need("SE_SPEED_DIR_FAST", "Ordner auf dem schnellen Laufwerk").map(PathBuf::from)
}

/// The second volume of the stage (`/…`).
pub fn slow_dir() -> Result<PathBuf, String> {
    need("SE_SPEED_DIR_SLOW", "Ordner auf dem zweiten Laufwerk").map(PathBuf::from)
}

/// A fresh, empty folder below `parent`.
pub fn fresh_dir(parent: &Path, name: &str) -> PathBuf {
    let folder = parent.join(unique(name));
    std::fs::create_dir_all(&folder).expect("create a fresh case folder");
    folder
}

/// Round trip of the shaped link in seconds (`SE_SPEED_RTT_MS`).
pub fn rtt_seconds() -> Option<f64> {
    var("SE_SPEED_RTT_MS")
        .and_then(|text| text.parse::<f64>().ok())
        .map(|ms| ms / 1000.0)
}

/// Runs a case body; `Err(reason)` is "nicht ausgeführt", a panic a failure.
pub fn guarded(name: &str, body: impl FnOnce() -> Result<(), String>) {
    if let Err(reason) = body() {
        not_run(name, &reason);
    }
}

/// A case coordinate must agree with the stage's `SE_SPEED_PROFILE`.
pub fn check_profile(case: &Case) -> Result<(), String> {
    let stage = need("SE_SPEED_PROFILE", "Strecke der Stufe")?;
    assert_eq!(
        stage, case.profile,
        "{}: die Stufe hat die Strecke {stage} gesetzt, der Fall misst {}",
        case.name, case.profile
    );
    Ok(())
}

pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ----------------------------------------------------------------- report

fn append_report(value: &Value) {
    let Some(path) = var("SE_SPEED_REPORT") else {
        return;
    };
    let mut line = value.to_string();
    line.push('\n');
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Announces a case that could not run (stdout line and report line).
pub fn not_run(name: &str, reason: &str) {
    let reason = one_line(reason);
    println!("SPEED_NOT_RUN {name} {reason}");
    append_report(&json!({"name": name, "status": "not_run", "reason": reason}));
}

/// The report line of a case (M-1). Rates share one `unit`: `B/s`, `files/s`.
#[derive(Clone, Debug, Default)]
pub struct ReportLine {
    pub name: String,
    pub way: String,
    pub direction: String,
    pub profile: String,
    pub size_bytes: u64,
    pub files: u64,
    pub unit: String,
    pub se_rate: f64,
    pub capacity: f64,
    pub share: f64,
    pub required: f64,
    pub requirement: String,
    pub reference_tool: String,
    pub reference_rate: f64,
    pub limiter: String,
    pub notes: Vec<String>,
    /// Requirements that could not run (tool missing); the case then counts
    /// as "nicht ausgeführt" although the other requirements held.
    pub skipped: Vec<String>,
}

impl ReportLine {
    pub fn new(case: &Case, unit: &str) -> Self {
        ReportLine {
            name: case.name.into(),
            way: case.way.into(),
            direction: case.direction.into(),
            profile: case.profile.into(),
            unit: unit.into(),
            ..ReportLine::default()
        }
    }

    /// Sets the capacity, the required share and its words.
    pub fn against(&mut self, capacity: f64, required: f64, requirement: &str) {
        self.capacity = capacity;
        self.required = required;
        self.requirement = requirement.into();
        self.share = if capacity > 0.0 {
            self.se_rate / capacity
        } else {
            0.0
        };
    }

    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name, "status": "measured", "way": self.way,
            "direction": self.direction, "profile": self.profile,
            "size_bytes": self.size_bytes, "files": self.files, "unit": self.unit,
            "se_rate": self.se_rate, "capacity": self.capacity, "share": self.share,
            "required": self.required, "requirement": self.requirement,
            "reference_tool": self.reference_tool, "reference_rate": self.reference_rate,
            "limiter": self.limiter, "notes": self.notes, "skipped": self.skipped,
        })
    }

    pub fn summary(&self) -> String {
        format!(
            "{} (Anteil {:.2} von {})",
            fmt_rate(self.se_rate, &self.unit),
            self.share,
            fmt_rate(self.capacity, &self.unit)
        )
    }
}

/// Appends the line; panics with `failure` when a requirement was missed;
/// announces the requirements that could not run.
pub fn finish(line: &ReportLine, failure: Option<&str>) {
    append_report(&line.to_json());
    println!("SPEED {}", line.to_json());
    if let Some(message) = failure {
        panic!("{message}");
    }
    if !line.skipped.is_empty() {
        println!(
            "SPEED_NOT_RUN {} {}",
            line.name,
            one_line(&line.skipped.join("; "))
        );
    }
}

pub fn fmt_rate(value: f64, unit: &str) -> String {
    if unit == "B/s" {
        format!("{:.2} MB/s", value / 1e6)
    } else {
        format!("{value:.1} {unit}")
    }
}

/// `None` when `measured` reaches `required × capacity`, else the failure text
/// of U55-3: `<name>: <value> < <required> × <capacity> (<capacity name> aus <source>)`.
pub fn shortfall(
    name: &str,
    measured: f64,
    capacity: f64,
    required: f64,
    capacity_name: &str,
    source: &str,
    unit: &str,
) -> Option<String> {
    if measured >= required * capacity {
        return None;
    }
    Some(format!(
        "{name}: {} < {required:.2} × {} ({capacity_name} aus {source})",
        fmt_rate(measured, unit),
        fmt_rate(capacity, unit)
    ))
}

pub fn assert_share(
    name: &str,
    measured: f64,
    capacity: f64,
    required: f64,
    capacity_name: &str,
    source: &str,
    unit: &str,
) {
    if let Some(message) = shortfall(
        name,
        measured,
        capacity,
        required,
        capacity_name,
        source,
        unit,
    ) {
        panic!("{message}");
    }
}

/// One attempt of a case: its report line and the shortfall, if any.
pub struct Trial {
    pub line: ReportLine,
    pub failure: Option<String>,
}

/// M-1: a shortfall is repeated once (shared runners are noisy). Both values
/// reach the report; a pass of the second attempt is marked "beim zweiten Lauf".
pub fn retry_once(mut attempt: impl FnMut() -> Trial) -> Trial {
    let first = attempt();
    let Some(first_failure) = first.failure.clone() else {
        return first;
    };
    let mut second = attempt();
    let earlier = first.line.summary();
    if second.failure.is_none() {
        second
            .line
            .notes
            .push(format!("beim zweiten Lauf (erster Lauf: {earlier})"));
    } else {
        second.line.notes.push(format!(
            "beide Läufe unterschritten (erster Lauf: {earlier})"
        ));
        second.failure = second
            .failure
            .take()
            .map(|message| format!("{message}; erster Lauf: {first_failure}"));
    }
    second
}

// ----------------------------------------------------------------- capacities

/// What `speed_task_ref_iperf_<profile>` wrote for a link, in bytes per second.
/// `tcp_*` is the loopback itself (FTP, Share), `proxy_tcp_*` the path through
/// Docker's published port (SFTP, WebDAV, SMB, agent).
#[derive(Clone, Debug, Default)]
pub struct Capacity {
    pub tcp_bps: f64,
    pub tcp1_bps: f64,
    pub udp_bps: f64,
    pub proxy_tcp_bps: f64,
    pub proxy_tcp1_bps: f64,
}

fn capacity_file() -> Result<(String, Value), String> {
    let path = need("SE_SPEED_CAPACITY", "Datei der Referenzfälle")?;
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
    let value = serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))?;
    Ok((path, value))
}

/// Reads one link's capacities; `Err` carries the reason of the reference
/// case that did not measure them.
pub fn capacity(profile: &str) -> Result<Capacity, String> {
    let (_, file) = capacity_file()?;
    let entry = file.get(profile).ok_or_else(|| {
        format!(
            "keine Kapazität der Strecke {profile} (speed_task_ref_iperf_{profile} nicht gelaufen)"
        )
    })?;
    if let Some(error) = entry.get("error").and_then(Value::as_str) {
        return Err(format!(
            "Kapazität der Strecke {profile} nicht gemessen: {error}"
        ));
    }
    let number = |key: &str| entry.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    Ok(Capacity {
        tcp_bps: number("tcp_bps"),
        tcp1_bps: number("tcp1_bps"),
        udp_bps: number("udp_bps"),
        proxy_tcp_bps: number("proxy_tcp_bps"),
        proxy_tcp1_bps: number("proxy_tcp1_bps"),
    })
}

/// Read, write and mixed rates of a volume (fio, M-3), bytes per second.
#[derive(Clone, Debug, Default)]
pub struct VolumeCapacity {
    pub read: f64,
    pub write: f64,
    pub mix: f64,
}

pub fn volume_capacity(dir: &Path) -> Result<VolumeCapacity, String> {
    let (_, file) = capacity_file()?;
    let key = dir.to_string_lossy().to_string();
    let entry = file
        .get("volumes")
        .and_then(|volumes| volumes.get(&key))
        .ok_or_else(|| {
            format!("keine fio-Kapazität des Laufwerks {key} (speed_task_ref_fio_volumes)")
        })?;
    let number = |name: &str| entry.get(name).and_then(Value::as_f64).unwrap_or(0.0);
    Ok(VolumeCapacity {
        read: number("read"),
        write: number("write"),
        mix: number("mix"),
    })
}

/// Files per second fio created in a folder (`F_fio`, M-3).
pub fn files_per_second(dir: &Path) -> Result<f64, String> {
    let (_, file) = capacity_file()?;
    let key = dir.to_string_lossy().to_string();
    file.get("files_per_s")
        .and_then(|rates| rates.get(&key))
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("keine fio-Dateirate für {key} (speed_task_ref_fio_small_files)"))
}

/// Merges `update` into the capacity file (created when absent).
pub fn update_capacity(update: impl FnOnce(&mut Value)) -> Result<(), String> {
    let path = need("SE_SPEED_CAPACITY", "Datei der Referenzfälle")?;
    let mut file: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));
    update(&mut file);
    std::fs::write(&path, file.to_string()).map_err(|error| format!("{path}: {error}"))
}

/// M-4: fifteen seconds of the capacity, between `min` and `max`, in whole MiB.
pub fn size_for(capacity_bps: f64, min: u64, max: u64) -> u64 {
    let wanted = (capacity_bps * 15.0) as u64;
    wanted.clamp(min, max).div_ceil(MIB) * MIB
}
