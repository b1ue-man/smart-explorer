//! `se screen codec` (SCR-M3): the encoders and decoders of this device,
//! the state of OpenH264 and, on Windows, of the FFmpeg runtime; `se screen
//! codec openh264 enable | disable | remove`. Since 2026-11-24 OpenH264
//! 2.6.0 from Cisco installs on its own (when the daemon starts and when a
//! session needs H.264 that no other coder has); its state reads
//! „wird installiert“, „installiert“, „abgeschaltet“ or „Fehler: <Grund> –
//! neuer Versuch um …“. Every output names „OpenH264 Video Codec provided by
//! Cisco Systems, Inc.“ and where Cisco's license is shown (Cisco's
//! conditions); `enable` needs no acceptance any more (`--yes` is still
//! accepted and changes nothing). `disable` switches OpenH264 off and no
//! automatic installation fetches it until `enable`; `remove` deletes the
//! library and switches it off as well. The state lives in this user's app
//! data, so the background worker sees a change within a second without a
//! request. JSON: the report with `openh264.state` (`absent`, `installing`,
//! `enabled`, `disabled`, `failed`), `reason`, `retry_at` (Unix seconds) and
//! `library_present`, and on Windows `ffmpeg_runtime` (`state`: `installed`,
//! `installing`, `absent`, `failed`; `version`, `reason`, `retry_at`).
//! `se screen codec licenses [NAME] [--json]` prints the embedded license
//! texts of the screen strand (SCR-M10, W6): without a name the list (key,
//! component, license), with a name the full text; JSON
//! `[{"key","component","license"}]` or `{"key","component","license","text"}`.
//! Exit codes: 0 success, 1 error with text (an `enable` whose download
//! failed: OpenH264 stays switched on and is tried again).
//!
//! Plan 2026-11-17 „Windows-Bild“ P3: every encoder and decoder row names
//! its run-time check (`verdict`: `verified`, `structural`, `failed` with
//! `reason`, `unchecked`; absent for the tiles). `se screen codec verify`
//! forgets the verdicts of this version and checks every encoder again with
//! a round trip of the test picture at 1280 × 720 through the local
//! decoders; `--json` prints `{"checks": [...], "codecs": {...}}`.
use clap::{Args, Subcommand};

use crate::screen::core::codec_registry::{CodecEntry, CodecReport, VerifyLine, OPENH264_NOTICE};
use crate::screen::core::codec_verdicts::VerdictView;
use crate::screen::os::codec_runtime_install::{
    ffmpeg_status, openh264_state_text, runtime_state_text, unix_now, RuntimeReport,
};
use crate::screen::os::{codec_inventory, openh264_action, openh264_license, OpenH264Action};

#[derive(Args, Debug, PartialEq, Eq)]
pub(super) struct CodecArgs {
    #[command(subcommand)]
    command: Option<CodecCommand>,
    #[arg(long, global = true, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
enum CodecCommand {
    #[command(
        about = "OpenH264 by Cisco (installed automatically): switch it on again, off, or remove it"
    )]
    Openh264 {
        #[command(subcommand)]
        action: OpenH264Command,
    },
    #[command(about = "License texts of the screen codecs (the full text with a name)")]
    Licenses {
        /// The license to print in full, e.g. openh264
        name: Option<String>,
    },
    #[command(
        about = "Check every encoder again with a round trip of the test picture through the local decoders"
    )]
    Verify,
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
enum OpenH264Command {
    #[command(about = "Switch OpenH264 on again and install it now when it is missing")]
    Enable {
        /// Accepted for older scripts; changes nothing
        #[arg(long, hide = true)]
        yes: bool,
    },
    #[command(about = "Switch OpenH264 off; no automatic installation fetches it until `enable`")]
    Disable,
    #[command(about = "Delete the OpenH264 library of this user and switch it off")]
    Remove,
}

pub(super) fn run(args: CodecArgs) -> Result<i32, String> {
    if let Some(CodecCommand::Licenses { name }) = &args.command {
        print!("{}", licenses(name.as_deref(), args.json)?);
        return Ok(0);
    }
    if let Some(CodecCommand::Verify) = &args.command {
        let (checks, report) = codec_inventory::verify();
        if args.json {
            let codecs = report_json(&report, ffmpeg_status().as_ref())?;
            let value = serde_json::json!({ "checks": checks, "codecs": codecs });
            let json = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
            println!("{json}");
        } else {
            let ffmpeg = runtime_text(ffmpeg_status().as_ref());
            print!("{}{}{ffmpeg}", checks_text(&checks), text(&report));
        }
        return Ok(0);
    }
    if let Some(CodecCommand::Openh264 { action }) = &args.command {
        let action = match action {
            OpenH264Command::Enable { .. } => OpenH264Action::Enable {
                accepted: openh264_license()
                    .ok_or("OpenH264 is not offered for this platform")?
                    .1
                    .to_string(),
            },
            OpenH264Command::Disable => OpenH264Action::Disable,
            OpenH264Command::Remove => OpenH264Action::Remove,
        };
        openh264_action(&action).map_err(|error| format!("OpenH264: {error}"))?;
    }
    let report = codec_inventory::report();
    let ffmpeg = ffmpeg_status();
    if args.json {
        let value = report_json(&report, ffmpeg.as_ref())?;
        let json = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
        println!("{json}");
    } else {
        print!("{}", text(&report));
        print!("{}", runtime_text(ffmpeg.as_ref()));
    }
    Ok(0)
}

/// The report as JSON, with `ffmpeg_runtime` where FFmpeg is fetched
/// (Windows).
fn report_json(
    report: &CodecReport,
    ffmpeg: Option<&RuntimeReport>,
) -> Result<serde_json::Value, String> {
    let mut value = serde_json::to_value(report).map_err(|error| error.to_string())?;
    if let (Some(ffmpeg), Some(object)) = (ffmpeg, value.as_object_mut()) {
        let ffmpeg = serde_json::to_value(ffmpeg).map_err(|error| error.to_string())?;
        object.insert("ffmpeg_runtime".into(), ffmpeg);
    }
    Ok(value)
}

/// The line of the FFmpeg runtime (Windows); empty elsewhere.
fn runtime_text(ffmpeg: Option<&RuntimeReport>) -> String {
    ffmpeg.map_or_else(String::new, |ffmpeg| {
        format!(
            "FFmpeg-Laufzeit {}: {}\n",
            ffmpeg.version,
            runtime_state_text(ffmpeg, unix_now())
        )
    })
}

/// The output of `se screen codec licenses [NAME]`.
pub(super) fn licenses(name: Option<&str>, json: bool) -> Result<String, String> {
    use crate::screen::core::licenses::{license, screen_licenses};
    let out = match name {
        None if json => {
            let list: Vec<serde_json::Value> = screen_licenses()
                .iter()
                .map(|entry| {
                    serde_json::json!({
                        "key": entry.key,
                        "component": entry.component,
                        "license": entry.license,
                    })
                })
                .collect();
            serde_json::to_string_pretty(&list).map_err(|error| error.to_string())? + "\n"
        }
        None => screen_licenses()
            .iter()
            .map(|entry| format!("{}\t{}\t{}\n", entry.key, entry.license, entry.component))
            .collect(),
        Some(name) => {
            let entry = license(name).ok_or_else(|| {
                format!("unknown license {name:?}; `se screen codec licenses` lists them")
            })?;
            if json {
                let value = serde_json::json!({
                    "key": entry.key,
                    "component": entry.component,
                    "license": entry.license,
                    "text": entry.text,
                });
                serde_json::to_string_pretty(&value).map_err(|error| error.to_string())? + "\n"
            } else {
                entry.text.to_string()
            }
        }
    };
    Ok(out)
}

/// The German words of a verdict (P3).
pub(super) fn verdict_text(verdict: &VerdictView) -> String {
    match (verdict.state.as_str(), verdict.reason.as_deref()) {
        ("verified", _) => "geprüft".into(),
        ("structural", _) => "Bitstrom geprüft (kein lokaler Dekodierer)".into(),
        ("failed", Some(reason)) => format!("Prüfung fehlgeschlagen: {reason}"),
        ("failed", None) => "Prüfung fehlgeschlagen".into(),
        _ => "ungeprüft".into(),
    }
}

/// The lines of `se screen codec verify`.
fn checks_text(checks: &[VerifyLine]) -> String {
    let mut out = String::from("Prüfung:\n");
    for check in checks {
        let label = format!("{} ({})", check.codec.key(), check.profile);
        let verdict = verdict_text(&check.verdict);
        out.push_str(&format!("  {label:<18} {}: {verdict}\n", check.encoder));
    }
    if checks.is_empty() {
        out.push_str("  kein Kodierer zu prüfen\n");
    }
    out
}

fn row(entry: &CodecEntry) -> String {
    // The names of the standard (BK6: HEVC `main`/`main_444`, AV1
    // `main`/`high`); a report of an older `se` has the wire keys only.
    let profiles: Vec<&str> = if entry.profile_names.is_empty() {
        entry.profiles.iter().map(|profile| profile.key()).collect()
    } else {
        entry.profile_names.iter().map(String::as_str).collect()
    };
    let codec = if profiles.is_empty() {
        entry.codec.key().to_string()
    } else {
        format!("{} ({})", entry.codec.key(), profiles.join(", "))
    };
    let kind = if entry.hardware {
        "Hardware"
    } else {
        "Software"
    };
    let refused = if entry.refused_settings.is_empty() {
        String::new()
    } else {
        format!("  abgelehnt: {}", entry.refused_settings.join(", "))
    };
    let verdict = entry
        .verdict
        .as_ref()
        .map(|verdict| format!("  [{}]", verdict_text(verdict)))
        .unwrap_or_default();
    format!(
        "  {codec:<18} {kind:<8}  {}{refused}{verdict}\n",
        entry.name
    )
}

/// The line of mode „Scharf (GPU)“ (plan 2026-11-17 P9): the GPU encoders
/// it uses on this device and whether one of them sends 4:4:4.
pub(super) fn sharp_text(report: &CodecReport) -> String {
    use crate::screen::core::sharp_mode::{SHARP_KEY, SHARP_LABEL};
    let encoders = report.sharp_encoders();
    if encoders.is_empty() {
        return format!(
            "{SHARP_LABEL} ({SHARP_KEY}): kein Video-Kodierer der Grafikkarte, \
             Software-Video oder Kacheln\n"
        );
    }
    let names: Vec<String> = encoders
        .iter()
        .map(|entry| format!("{} {}", entry.codec.key(), entry.name))
        .collect();
    let chroma = if report.sharp_444() {
        "4:4:4, wo der Betrachter es dekodiert"
    } else {
        "4:2:0"
    };
    format!(
        "{SHARP_LABEL} ({SHARP_KEY}): {} ({chroma})\n",
        names.join(", ")
    )
}

/// The text of `se screen codec`.
pub(super) fn text(report: &CodecReport) -> String {
    let mut out = String::from("Kodierer:\n");
    report
        .encoders
        .iter()
        .for_each(|entry| out.push_str(&row(entry)));
    out.push_str("Dekodierer:\n");
    report
        .decoders
        .iter()
        .for_each(|entry| out.push_str(&row(entry)));
    out.push_str(&sharp_text(report));
    let state = openh264_state_text(&report.openh264, unix_now());
    out.push_str(&format!("OpenH264 {}: {state}\n", report.openh264.version));
    out.push_str(OPENH264_NOTICE);
    out.push_str(" – Lizenz: se screen codec licenses openh264\n");
    out
}

#[cfg(test)]
mod sharp_tests {
    use super::{sharp_text, text};
    use crate::screen::core::codec_registry::{
        CodecEntry, CodecReport, OpenH264Report, OpenH264State,
    };
    use crate::screen::core::negotiate::CodecKind;

    fn entry(codec: CodecKind, name: &str, hardware: bool, profiles: &[&str]) -> CodecEntry {
        CodecEntry {
            codec,
            profiles: Vec::new(),
            name: name.into(),
            hardware,
            profile_names: profiles.iter().map(|p| p.to_string()).collect(),
            refused_settings: Vec::new(),
            verdict: None,
        }
    }

    #[test]
    fn codec_runtime_task_cli_codec_names_state_notice_and_license() {
        let mut report = CodecReport {
            encoders: Vec::new(),
            decoders: Vec::new(),
            openh264: OpenH264Report::new(OpenH264State::Installing, None),
        };
        let shown = text(&report);
        assert!(
            shown.contains("OpenH264 2.6.0: wird installiert\n"),
            "{shown}"
        );
        assert!(shown.contains(
            "OpenH264 Video Codec provided by Cisco Systems, Inc. – Lizenz: \
             se screen codec licenses openh264\n"
        ));
        report.openh264 = OpenH264Report::new(OpenH264State::Failed, Some("no answer".into()))
            .with_retry_at(Some(i64::MAX / 2));
        assert!(text(&report).contains("OpenH264 2.6.0: Fehler: no answer – neuer Versuch um "));
        let ffmpeg = super::RuntimeReport {
            state: crate::screen::os::codec_runtime_install::RuntimeState::Installed,
            version: "n8.1".into(),
            reason: None,
            retry_at: None,
        };
        assert_eq!(
            super::runtime_text(Some(&ffmpeg)),
            "FFmpeg-Laufzeit n8.1: installiert\n"
        );
        assert_eq!(super::runtime_text(None), "");
        let json = super::report_json(&report, Some(&ffmpeg)).unwrap();
        assert_eq!(json["ffmpeg_runtime"]["state"], "installed");
        assert_eq!(json["openh264"]["state"], "failed");
        assert_eq!(json["openh264"]["reason"], "no answer");
        assert!(super::report_json(&report, None)
            .unwrap()
            .get("ffmpeg_runtime")
            .is_none());
    }

    #[test]
    fn sharp_mode_task_cli_codec_names_the_mode() {
        let mut report = CodecReport {
            encoders: vec![
                entry(CodecKind::Av1, "nvenc-av1", true, &["main"]),
                entry(CodecKind::Hevc, "nvenc-hevc", true, &["main", "main_444"]),
                entry(CodecKind::H264, "openh264", false, &["cb"]),
            ],
            decoders: Vec::new(),
            openh264: OpenH264Report::new(OpenH264State::Absent, None),
        };
        assert_eq!(
            sharp_text(&report),
            "Scharf (GPU) (sharp): hevc nvenc-hevc (4:4:4, wo der Betrachter es dekodiert)\n"
        );
        assert!(text(&report).contains("Scharf (GPU) (sharp): hevc nvenc-hevc"));
        report.encoders.retain(|e| !e.hardware);
        assert!(sharp_text(&report).contains("kein Video-Kodierer der Grafikkarte"));
    }
}

#[cfg(test)]
mod licenses_tests {
    use super::licenses;
    use crate::screen::core::licenses::CISCO_BINARY_LICENSE;

    #[test]
    fn screen_task_codec_licenses_lists_and_prints() {
        let list = licenses(None, false).unwrap();
        assert!(
            list.starts_with("openh264\tCisco binary license v1.0\t"),
            "{list}"
        );
        let json: serde_json::Value = serde_json::from_str(&licenses(None, true).unwrap()).unwrap();
        assert_eq!(json[0]["key"], "openh264");
        assert!(json[0].get("text").is_none());
        assert_eq!(
            licenses(Some("openh264"), false).unwrap(),
            CISCO_BINARY_LICENSE
        );
        let full: serde_json::Value =
            serde_json::from_str(&licenses(Some("openh264"), true).unwrap()).unwrap();
        assert_eq!(full["text"], CISCO_BINARY_LICENSE);
        assert!(licenses(Some("x264"), false).is_err());
    }
}
