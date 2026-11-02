//! Text and JSON of `se screen` (SCR-M2). Text lines are tab-separated
//! `key<TAB>value` like the other `se` commands; JSON follows the contracts
//! of the strand plan: `probe` `{"device","supported","platform","view",
//! "control","monitors","input","codecs","limits"}`, `bench` `{"codec",
//! "encoder","decoder","path","frames_sent","frames_shown","fps_mean",
//! "fps_min","latency_ms":{"p50","p95","max"},"bitrate_kbps":{"mean","max"},
//! "keyframes","recoveries","corrupt_frames","static_psnr_db"}` (`null` =
//! infinite), `sessions` `{"sessions":[…]}`, `stop` `{"stopped"}` and
//! `devices` `{"devices":[{"target","device_id","name","relation","room",
//! "online","probe"}]}`.
use serde::Serialize;
use serde_json::{json, Value};

use super::DeviceChoice;
use crate::daemon::ScreenProbeReport;
use crate::screen::core::bench::BenchReport;
use crate::screen::core::registry::SessionView;

/// JSON (pretty) or the text of `text`.
pub(super) fn print<T: Serialize>(json: bool, value: &T, text: impl FnOnce() -> String) {
    if json {
        let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
        println!("{rendered}");
    } else {
        print!("{}", text());
    }
}

fn lines(lines: Vec<String>) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn listed(values: &[String]) -> String {
    if values.is_empty() {
        "-".into()
    } else {
        values.join(",")
    }
}

pub(super) fn probe_text(report: &ScreenProbeReport) -> String {
    let platform = report.platform.map_or("-".to_string(), |platform| {
        json!(platform).as_str().unwrap_or("-").to_string()
    });
    let codecs: Vec<String> = report
        .codecs
        .iter()
        .map(|offer| match offer.profile {
            Some(profile) => format!("{}:{}", offer.codec.key(), profile.key()),
            None => offer.codec.key().to_string(),
        })
        .collect();
    let supported = if report.supported { "yes" } else { "no" };
    lines(vec![
        format!("screen\t{}\tsupported={supported}", report.device),
        format!("platform\t{platform}"),
        format!("view\t{}", report.view.key()),
        format!("control\t{}", report.control.key()),
        format!("codecs\t{}", listed(&codecs)),
        format!("monitors\t{}", report.monitors.len()),
        format!("limits\t{}", listed(&report.limits)),
    ])
}

fn number(value: f64) -> String {
    format!("{value:.1}")
}

pub(super) fn bench_text(report: &BenchReport) -> String {
    let recoveries: Vec<String> = report
        .recoveries
        .iter()
        .map(|value| number(*value))
        .collect();
    let psnr = report.static_psnr_db.map_or("inf".to_string(), number);
    lines(vec![
        format!(
            "bench\tcodec={}\tencoder={}\tdecoder={}\tpath={}",
            report.codec, report.encoder, report.decoder, report.path
        ),
        format!(
            "frames\tsent={}\tshown={}\tcorrupt={}\tkeyframes={}",
            report.frames_sent, report.frames_shown, report.corrupt_frames, report.keyframes
        ),
        format!(
            "fps\tmean={}\tmin={}",
            number(report.fps_mean),
            number(report.fps_min)
        ),
        format!(
            "latency_ms\tp50={}\tp95={}\tmax={}",
            number(report.latency_ms.p50),
            number(report.latency_ms.p95),
            number(report.latency_ms.max)
        ),
        format!(
            "bitrate_kbps\tmean={}\tmax={}",
            number(report.bitrate_kbps.mean),
            number(report.bitrate_kbps.max)
        ),
        format!("recoveries_ms\t{}", listed(&recoveries)),
        format!("static_psnr_db\t{psnr}"),
    ])
}

pub(super) fn sessions_json(sessions: &[SessionView]) -> Value {
    json!({ "sessions": sessions })
}

pub(super) fn sessions_text(sessions: &[SessionView]) -> String {
    let mut rows = vec![format!("sessions\t{}", sessions.len())];
    rows.extend(sessions.iter().map(|session| {
        let controlling = if session.controlling { "yes" } else { "no" };
        [
            "session".to_string(),
            session.id.clone(),
            session.device_name.clone(),
            session.relation.clone(),
            session.room_id.clone().unwrap_or_else(|| "-".into()),
            session.mode.key().to_string(),
            session.source.clone(),
            session.state.key().to_string(),
            session.codec.clone().unwrap_or_else(|| "-".into()),
            session.path.clone(),
            format!("{}fps", session.fps),
            format!("{}kbps", session.bitrate_kbps),
            format!("controlling={controlling}"),
        ]
        .join("\t")
    }));
    lines(rows)
}

pub(super) fn stop_json(count: usize) -> Value {
    json!({ "stopped": count })
}

/// One device with the result of its probe (`--probe`).
type Probed = Option<Result<ScreenProbeReport, String>>;

pub(super) fn devices_json(choices: &[DeviceChoice], probes: &[Probed]) -> Value {
    let devices: Vec<Value> = choices
        .iter()
        .zip(probes)
        .map(|(choice, probe)| {
            let probe = match probe {
                Some(Ok(report)) => serde_json::to_value(report).unwrap_or(Value::Null),
                Some(Err(error)) => json!({ "error": error }),
                None => Value::Null,
            };
            json!({
                "target": choice.endpoint,
                "device_id": choice.device_id,
                "name": choice.name,
                "relation": choice.relation,
                "room": choice.room,
                "online": choice.online,
                "probe": probe,
            })
        })
        .collect();
    json!({ "devices": devices })
}

pub(super) fn devices_text(choices: &[DeviceChoice], probes: &[Probed]) -> String {
    let mut rows = vec![format!("devices\t{}", choices.len())];
    rows.extend(choices.iter().zip(probes).map(|(choice, probe)| {
        let room = choice.room.as_deref().unwrap_or("-");
        let online = if choice.online { "online" } else { "offline" };
        let facts = match probe {
            Some(Ok(report)) if report.supported => {
                format!(
                    "\tview={}\tcontrol={}",
                    report.view.key(),
                    report.control.key()
                )
            }
            Some(Ok(_)) => "\tscreen=unsupported".to_string(),
            Some(Err(error)) => format!("\tprobe_error={}", error.replace(['\t', '\n'], " ")),
            None => String::new(),
        };
        format!(
            "device\t{}\t{}\t{}\t{room}\t{online}{facts}",
            choice.endpoint, choice.name, choice.relation
        )
    }));
    lines(rows)
}
