//! `se screen codec` (SCR-M3): the encoders and decoders of this device and
//! the state of OpenH264; `se screen codec openh264 enable [--yes] |
//! disable | remove`. Before every download the full Cisco license is shown
//! (stdout, with `--json` on stderr); without `--yes` the command asks on a
//! terminal and refuses without one. Switching names „OpenH264 Video Codec
//! provided by Cisco Systems, Inc.“ (Cisco condition 3). The state lives in
//! this user's app data, so the background worker sees a change within a
//! second without a request. `se screen codec licenses [NAME] [--json]`
//! prints the embedded license texts of the screen strand (SCR-M10, W6):
//! without a name the list (key, component, license), with a name the full
//! text; JSON `[{"key","component","license"}]` or
//! `{"key","component","license","text"}`. Exit codes: 0 success, 1 error
//! with text.
use std::io::{IsTerminal, Write};

use clap::{Args, Subcommand};

use crate::screen::core::codec_registry::{
    CodecEntry, CodecReport, OpenH264State, OPENH264_NOTICE,
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
    #[command(about = "OpenH264 by Cisco: enable (download after the license), disable, remove")]
    Openh264 {
        #[command(subcommand)]
        action: OpenH264Command,
    },
    #[command(about = "License texts of the screen codecs (the full text with a name)")]
    Licenses {
        /// The license to print in full, e.g. openh264
        name: Option<String>,
    },
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
enum OpenH264Command {
    #[command(about = "Show Cisco's license, download OpenH264 2.6.0 and switch it on")]
    Enable {
        /// Accept the Cisco license shown before the download
        #[arg(long)]
        yes: bool,
    },
    #[command(about = "Switch OpenH264 off; the file stays for switching on again")]
    Disable,
    #[command(about = "Delete the OpenH264 library of this user")]
    Remove,
}

pub(super) fn run(args: CodecArgs) -> Result<i32, String> {
    if let Some(CodecCommand::Licenses { name }) = &args.command {
        print!("{}", licenses(name.as_deref(), args.json)?);
        return Ok(0);
    }
    if let Some(CodecCommand::Openh264 { action }) = &args.command {
        let action = match action {
            OpenH264Command::Enable { yes } => OpenH264Action::Enable {
                accepted: accept(*yes, args.json)?,
            },
            OpenH264Command::Disable => OpenH264Action::Disable,
            OpenH264Command::Remove => OpenH264Action::Remove,
        };
        openh264_action(&action).map_err(|error| format!("OpenH264: {error}"))?;
    }
    let report = codec_inventory::report();
    if args.json {
        let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
        println!("{json}");
    } else {
        print!("{}", text(&report));
    }
    Ok(0)
}

/// Shows the Cisco license and returns the SHA-256 of the accepted text.
fn accept(yes: bool, json: bool) -> Result<String, String> {
    let (license, sha256) =
        openh264_license().ok_or("OpenH264 is not offered for this platform")?;
    if json {
        eprintln!("{license}");
    } else {
        println!("{license}");
    }
    if yes {
        return Ok(sha256.to_string());
    }
    if !std::io::stdin().is_terminal() {
        return Err(
            "read the license above; accept it with `se screen codec openh264 enable --yes`".into(),
        );
    }
    eprint!("Lizenz annehmen und OpenH264 2.6.0 von Cisco herunterladen? [j/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| error.to_string())?;
    match answer.trim().to_lowercase().as_str() {
        "j" | "ja" | "y" | "yes" => Ok(sha256.to_string()),
        _ => Err("license not accepted; nothing was downloaded".into()),
    }
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

fn row(entry: &CodecEntry) -> String {
    let profiles: Vec<&str> = entry.profiles.iter().map(|profile| profile.key()).collect();
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
    format!("  {codec:<18} {kind:<8}  {}\n", entry.name)
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
    let state = match report.openh264.state {
        OpenH264State::Absent => "nicht geladen",
        OpenH264State::Enabled => "an",
        OpenH264State::Disabled => "aus",
        OpenH264State::Failed => "fehlerhaft",
    };
    out.push_str(&format!("OpenH264 {}: {state}", report.openh264.version));
    if let Some(reason) = &report.openh264.reason {
        out.push_str(&format!(" ({reason})"));
    }
    out.push('\n');
    out.push_str(OPENH264_NOTICE);
    out.push('\n');
    out
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
