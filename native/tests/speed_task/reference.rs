//! Capacity references (U55-4, M-3): `iperf3` for the lines, `fio` for the
//! drives, the tool inventory, and the `rclone` comparison of the small-file
//! cases. The seven `speed_task_ref_*` cases write `SE_SPEED_CAPACITY`, which
//! every other case reads; a reference that cannot be measured stores its
//! error text there so the dependent cases report the exact reason.
#![allow(dead_code)]

use crate::remote::{Server, Way};
use crate::support::{
    files_per_second, finish, guarded, need, not_run, update_capacity, var, Case, ReportLine,
};
use crate::tools::{exec_missing, run_tool, strings};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const IPERF_LIMIT: Duration = Duration::from_secs(120);
const FIO_LIMIT: Duration = Duration::from_secs(300);

fn parse_json(text: &str) -> Result<Value, String> {
    let start = text.find('{').ok_or("keine JSON-Ausgabe")?;
    serde_json::from_str(&text[start..]).map_err(|error| format!("JSON-Ausgabe: {error}"))
}

// ----------------------------------------------------------------- iperf3

/// One `iperf3` client run against `127.0.0.1:port`; the server is the stage's.
fn iperf(port: &str, extra: &[&str]) -> Result<Value, String> {
    let mut args = strings(&["-c", "127.0.0.1", "-p", port, "-t", "10", "-J"]);
    args.extend(extra.iter().map(|item| item.to_string()));
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(3));
        }
        let done = run_tool("iperf3", &args, &[], IPERF_LIMIT)?;
        if done.ok() {
            return parse_json(&done.stdout);
        }
        last = format!("iperf3 {}: {}", args.join(" "), done.describe());
    }
    Err(last)
}

fn received_bytes(result: &Value) -> Result<f64, String> {
    result["end"]["sum_received"]["bits_per_second"]
        .as_f64()
        .map(|bits| bits / 8.0)
        .ok_or_else(|| "iperf3: end.sum_received.bits_per_second fehlt".to_string())
}

/// The capacity entry of one link (M-3): TCP with 8 and with 1 stream on the
/// loopback, UDP at the link's rate, and both TCP values again through the
/// published port when the stage runs a proxied iperf3 server.
fn measure_link(rate_mbit: &str, host: &str, proxy: Option<&str>) -> Result<Value, String> {
    let tcp = received_bytes(&iperf(host, &["-P", "8"])?)?;
    let tcp1 = received_bytes(&iperf(host, &["-P", "1"])?)?;
    let bitrate = format!("{rate_mbit}M");
    let udp = iperf(host, &["-u", "-b", bitrate.as_str()])?;
    let sum = &udp["end"]["sum"];
    let lost = sum["lost_percent"].as_f64().unwrap_or(0.0);
    let udp_bytes = sum["bits_per_second"]
        .as_f64()
        .ok_or("iperf3 -u: end.sum.bits_per_second fehlt")?
        / 8.0
        * (1.0 - lost / 100.0);
    let mut entry = json!({"tcp_bps": tcp, "tcp1_bps": tcp1, "udp_bps": udp_bytes,
        "udp_lost_percent": lost});
    if let Some(port) = proxy {
        entry["proxy_tcp_bps"] = json!(received_bytes(&iperf(port, &["-P", "8"])?)?);
        entry["proxy_tcp1_bps"] = json!(received_bytes(&iperf(port, &["-P", "1"])?)?);
    }
    Ok(entry)
}

fn ref_iperf(name: &'static str, profile: &'static str) {
    guarded(name, || {
        let stage = need("SE_SPEED_PROFILE", "Strecke der Stufe")?;
        assert_eq!(stage, profile, "{name}: die Stufe misst {stage}");
        let rate = need("SE_SPEED_RATE_MBIT", "Rate der Strecke")?;
        let host = need("SE_SPEED_IPERF_HOST_PORT", "iperf3-Server auf der Schleife")?;
        let proxy = var("SE_SPEED_IPERF_PROXY_PORT");
        let measured = measure_link(&rate, &host, proxy.as_deref());
        let case = Case::new(name, "iperf3", "both", profile);
        let mut line = ReportLine::new(&case, "B/s");
        line.reference_tool = "iperf3 -P 8 / -P 1 / -u".into();
        match measured {
            Ok(entry) => {
                line.se_rate = entry["tcp_bps"].as_f64().unwrap_or(0.0);
                line.capacity = line.se_rate;
                line.share = 1.0;
                line.requirement = "Messwert (keine Forderung)".into();
                line.notes.push(format!("{entry}"));
                update_capacity(|file| file[profile] = entry.clone())?;
                finish(&line, None);
                Ok(())
            }
            Err(error) => {
                update_capacity(|file| file[profile] = json!({"error": error.clone()}))?;
                if error.contains("nicht startbar") {
                    return Err(error);
                }
                panic!("{name}: {error}");
            }
        }
    });
}

#[test]
#[ignore = "needs the stage's iperf3 servers and shaped loopback"]
fn speed_task_ref_iperf_lan() {
    ref_iperf("speed_task_ref_iperf_lan", "lan");
}

#[test]
#[ignore = "needs the stage's iperf3 servers and shaped loopback"]
fn speed_task_ref_iperf_wan() {
    ref_iperf("speed_task_ref_iperf_wan", "wan");
}

#[test]
#[ignore = "needs the stage's iperf3 servers and shaped loopback"]
fn speed_task_ref_iperf_fat() {
    ref_iperf("speed_task_ref_iperf_fat", "fat");
}

#[test]
#[ignore = "needs the stage's iperf3 servers and shaped loopback"]
fn speed_task_ref_iperf_loss() {
    ref_iperf("speed_task_ref_iperf_loss", "loss");
}

// ----------------------------------------------------------------- fio

fn fio_path(dir: &Path) -> String {
    let text = dir.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text.replace(':', "\\:")
    } else {
        text
    }
}

fn ioengine() -> &'static str {
    if cfg!(windows) {
        "windowsaio"
    } else {
        "libaio"
    }
}

fn fio_bytes(result: &Value, section: &str) -> f64 {
    result["jobs"]
        .as_array()
        .map(|jobs| {
            jobs.iter()
                .map(|job| {
                    let side = &job[section];
                    side["bw_bytes"]
                        .as_f64()
                        .or_else(|| side["bw"].as_f64().map(|kib| kib * 1024.0))
                        .unwrap_or(0.0)
                })
                .sum()
        })
        .unwrap_or(0.0)
}

fn fio(args: Vec<String>) -> Result<Value, String> {
    let done = run_tool("fio", &args, &[], FIO_LIMIT)?;
    if !done.ok() {
        return Err(format!("fio {}: {}", args.join(" "), done.describe()));
    }
    parse_json(&done.stdout)
}

/// Read, write and mixed rate of the volume holding `dir` (M-3 wording).
pub fn run_fio(dir: &Path) -> Result<(f64, f64, f64), String> {
    let mut rates = Vec::new();
    for mode in ["read", "write", "rw"] {
        let mut args = strings(&[
            "--name=speedvol",
            "--filename=speed-fio.bin",
            "--size=4G",
            "--bs=1M",
            "--iodepth=16",
            "--direct=1",
            "--rwmixread=50",
            "--runtime=20",
            "--time_based=0",
            "--output-format=json",
        ]);
        args.push(format!("--directory={}", fio_path(dir)));
        args.push(format!("--ioengine={}", ioengine()));
        args.push(format!("--rw={mode}"));
        let result = fio(args)?;
        rates.push(match mode {
            "read" => fio_bytes(&result, "read"),
            "write" => fio_bytes(&result, "write"),
            _ => fio_bytes(&result, "read") + fio_bytes(&result, "write"),
        });
    }
    let _ = std::fs::remove_file(dir.join("speed-fio.bin"));
    Ok((rates[0], rates[1], rates[2]))
}

/// Files per second fio creates in a folder of the volume (M-3 wording).
pub fn run_fio_files(dir: &Path) -> Result<f64, String> {
    let folder = dir.join("speed-fio-files");
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let cores = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(2);
    let mut args = strings(&[
        "--name=files",
        "--rw=write",
        "--bs=4k",
        "--filesize=4k",
        "--nrfiles=5000",
        "--create_on_open=1",
        "--direct=0",
        "--fallocate=none",
        "--output-format=json",
    ]);
    args.push(format!("--directory={}", fio_path(&folder)));
    args.push(format!("--numjobs={cores}"));
    let result = fio(args);
    let _ = std::fs::remove_dir_all(&folder);
    let result = result?;
    let jobs = result["jobs"].as_array().ok_or("fio: keine Jobs")?;
    let ios: f64 = jobs
        .iter()
        .map(|job| job["write"]["total_ios"].as_f64().unwrap_or(0.0))
        .sum();
    let runtime_ms = jobs
        .iter()
        .map(|job| job["job_runtime"].as_f64().unwrap_or(0.0))
        .fold(0.0, f64::max);
    if runtime_ms <= 0.0 {
        return Err("fio: job_runtime fehlt".into());
    }
    Ok(ios / (runtime_ms / 1000.0))
}

/// The distinct drive folders of the stage (fast first).
fn drive_dirs() -> Result<Vec<PathBuf>, String> {
    let mut dirs = vec![PathBuf::from(need(
        "SE_SPEED_DIR_FAST",
        "Ordner auf dem schnellen Laufwerk",
    )?)];
    if let Some(slow) = var("SE_SPEED_DIR_SLOW") {
        if !dirs.contains(&PathBuf::from(&slow)) {
            dirs.push(PathBuf::from(slow));
        }
    }
    Ok(dirs)
}

#[test]
#[ignore = "needs fio and the stage's drive folders"]
fn speed_task_ref_fio_volumes() {
    let name = "speed_task_ref_fio_volumes";
    guarded(name, || {
        let case = Case::new(name, "fio", "both", "drive");
        let mut line = ReportLine::new(&case, "B/s");
        line.reference_tool = "fio --bs=1M --iodepth=16 --direct=1".into();
        line.requirement = "Messwert (keine Forderung)".into();
        for dir in drive_dirs()? {
            let (read, write, mix) = run_fio(&dir)?;
            let key = dir.to_string_lossy().to_string();
            update_capacity(|file| {
                file["volumes"][key.as_str()] = json!({"read": read, "write": write, "mix": mix})
            })?;
            line.notes.push(format!(
                "{key}: Lesen {:.0} MB/s, Schreiben {:.0} MB/s, gemischt {:.0} MB/s",
                read / 1e6,
                write / 1e6,
                mix / 1e6
            ));
            line.se_rate = line.se_rate.max(read);
        }
        line.capacity = line.se_rate;
        line.share = 1.0;
        finish(&line, None);
        Ok(())
    });
}

#[test]
#[ignore = "needs fio and the stage's drive folders"]
fn speed_task_ref_fio_small_files() {
    let name = "speed_task_ref_fio_small_files";
    guarded(name, || {
        let case = Case::new(name, "fio", "write", "drive");
        let mut line = ReportLine::new(&case, "files/s");
        line.reference_tool = "fio --nrfiles=5000 --filesize=4k".into();
        line.requirement = "Messwert (keine Forderung)".into();
        for dir in drive_dirs()? {
            let rate = run_fio_files(&dir)?;
            let key = dir.to_string_lossy().to_string();
            update_capacity(|file| file["files_per_s"][key.as_str()] = json!(rate))?;
            line.notes.push(format!("{key}: {rate:.0} Dateien/s"));
            line.se_rate = line.se_rate.max(rate);
        }
        line.capacity = line.se_rate;
        line.share = 1.0;
        finish(&line, None);
        Ok(())
    });
}

// ----------------------------------------------------------------- tools

/// The programs a job needs: `SE_SPEED_TOOLS_REQUIRED` (comma separated) or
/// the platform's set. Each is asked for its version.
fn required_tools() -> Vec<String> {
    if let Some(list) = var("SE_SPEED_TOOLS_REQUIRED") {
        return list
            .split(',')
            .map(|item| item.trim().to_string())
            .collect();
    }
    let set: &[&str] = if cfg!(windows) {
        &["fio", "robocopy"]
    } else {
        &[
            "fio", "iperf3", "tc", "rsync", "cp", "curl", "ssh", "docker",
        ]
    };
    set.iter().map(|item| item.to_string()).collect()
}

fn version_arguments(tool: &str) -> Vec<String> {
    match tool {
        "ssh" => strings(&["-V"]),
        "tc" => strings(&["-V"]),
        "robocopy" => strings(&["/?"]),
        _ => strings(&["--version"]),
    }
}

#[test]
#[ignore = "needs the reference tools of the stage"]
fn speed_task_ref_tools_present() {
    let name = "speed_task_ref_tools_present";
    guarded(name, || {
        let mut versions = serde_json::Map::new();
        let mut missing = Vec::new();
        for tool in required_tools() {
            match exec_missing(&tool, &version_arguments(&tool)) {
                Ok(text) => {
                    let first = text.lines().next().unwrap_or("").trim().to_string();
                    versions.insert(tool, json!(first));
                }
                Err(error) => {
                    versions.insert(tool.clone(), Value::Null);
                    missing.push(format!("{tool} ({error})"));
                }
            }
        }
        match var("SE_SPEED_RCLONE") {
            Some(path) => match exec_missing(&path, &strings(&["version"])) {
                Ok(text) => {
                    versions.insert("rclone".into(), json!(text.lines().next().unwrap_or("")));
                }
                Err(error) => missing.push(format!("rclone ({error})")),
            },
            None => {
                versions.insert("rclone".into(), Value::Null);
            }
        }
        if let Some(path) = var("SE_SPEED_TOOLS_JSON") {
            let _ = std::fs::write(path, Value::Object(versions.clone()).to_string());
        }
        let case = Case::new(name, "tools", "-", "-");
        let mut line = ReportLine::new(&case, "B/s");
        line.requirement = "alle Referenzwerkzeuge vorhanden".into();
        line.notes.push(Value::Object(versions).to_string());
        finish(&line, None);
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!("Referenzwerkzeug fehlt: {}", missing.join(", ")))
        }
    });
}

// ----------------------------------------------------------------- rclone

/// Files per second `rclone copy` reaches on the same data and server (M-5):
/// `--no-traverse --ignore-times`, `--transfers` = `--checkers` = `transfers`.
/// `upload`: `local` is copied to `remote_path`; else the reverse.
pub fn rclone_files_per_second(
    way: Way,
    server: &Server,
    local: &Path,
    remote_path: &str,
    upload: bool,
    transfers: u32,
    files: u64,
) -> Result<f64, String> {
    let binary = var("SE_SPEED_RCLONE").ok_or(
        "rclone nicht verfügbar (SE_SPEED_RCLONE fehlt: Archiv nicht geladen oder SHA-256 abweichend)",
    )?;
    let kind = match way {
        Way::Sftp | Way::Agent => "sftp",
        Way::Ftp => "ftp",
        Way::Webdav => "webdav",
        Way::Smb => "smb",
        Way::Share => return Err("rclone kennt das Share-Protokoll nicht".into()),
    };
    let obscured = run_tool(
        &binary,
        &strings(&["obscure", &server.password]),
        &[],
        IPERF_LIMIT,
    )?;
    if !obscured.ok() {
        return Err(format!("rclone obscure: {}", obscured.describe()));
    }
    let config = std::env::temp_dir().join("speed-rclone.conf");
    let set = |key: &str, value: String| (format!("RCLONE_CONFIG_SPEED_{key}"), value);
    let mut envs = vec![
        (
            "RCLONE_CONFIG".to_string(),
            config.to_string_lossy().to_string(),
        ),
        set("TYPE", kind.into()),
        set("USER", server.user.clone()),
        set("PASS", obscured.stdout.trim().to_string()),
    ];
    if way == Way::Webdav {
        envs.push(set(
            "URL",
            format!("http://{}:{}", server.host, server.port),
        ));
        envs.push(set("VENDOR", "other".into()));
    } else {
        envs.push(set("HOST", server.host.clone()));
        envs.push(set("PORT", server.port.to_string()));
    }
    // SMB paths start with the share name, the others with the server root.
    let remote = if way == Way::Smb {
        format!("speed:{}", remote_path.trim_start_matches('/'))
    } else {
        format!("speed:{remote_path}")
    };
    let local_text = local.to_string_lossy().to_string();
    let (from, to) = if upload {
        (local_text, remote)
    } else {
        (remote, local_text)
    };
    let mut args = strings(&["copy", "--no-traverse", "--ignore-times"]);
    args.push(format!("--transfers={transfers}"));
    args.push(format!("--checkers={transfers}"));
    args.push(from);
    args.push(to);
    let done = run_tool(&binary, &args, &envs, Duration::from_secs(600))?;
    if !done.ok() {
        return Err(format!("rclone copy: {}", done.describe()));
    }
    Ok(files as f64 / done.wall.as_secs_f64().max(0.001))
}

/// Records a skipped requirement against `rclone` in the report line.
pub fn rclone_unavailable(line: &mut ReportLine, reason: &str) {
    line.skipped
        .push(format!("Forderung gegen rclone nicht ausgeführt: {reason}"));
}

/// The names of the cases for `not_run` without a body (kept for the stage).
pub fn announce_missing(name: &str, reason: &str) {
    not_run(name, reason);
}

/// F_fio of a folder, or the reason the reference is missing.
pub fn fio_files(dir: &Path) -> Result<f64, String> {
    files_per_second(dir)
}
