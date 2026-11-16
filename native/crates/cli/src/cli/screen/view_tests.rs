//! Acceptance of `se screen view` and `se screen grid` (SCR-M7): the command
//! line, the target JSON they hand to the GUI program and its way through the
//! start parser (V5, V9) to the viewer's `ScreenTarget`.
use clap::Parser;
use serde_json::json;

use super::{target_json, ViewArgs};
use crate::app::screen_target::{Relation, ScreenTarget};
use crate::app::start_request::{self, StartRequest};
use crate::cli::screen::{device_choices, select_from, ScreenCommand};
use crate::share::{DirectContact, PeerOpenTarget, RoomProfile, ShareProfiles};

#[derive(Parser)]
struct Wrapper {
    #[command(subcommand)]
    command: ScreenCommand,
}

fn contact(id: &str, name: &str, device_id: Option<&str>) -> DirectContact {
    serde_json::from_value(json!({
        "id": id,
        "display_name": name,
        "lookup_id": format!("lookup-{id}"),
        "expected_fingerprint": format!("fp-{id}"),
        "remote_device_id": device_id,
        "auto_connect": false,
        "auto_open": false,
    }))
    .unwrap()
}

fn room() -> RoomProfile {
    serde_json::from_value(json!({
        "id": "profile-1",
        "name": "Ops",
        "room_id": "room-1",
        "auto_join": true,
        "last_seen": null,
        "members": [{
            "device_id": "device-m1",
            "device_name": "Rack",
            "fingerprint": "fp-m1",
            "public_key": "key-m1",
            "candidates": [],
            "last_seen": null,
        }],
    }))
    .unwrap()
}

fn profiles() -> ShareProfiles {
    ShareProfiles {
        direct_contacts: vec![
            contact("c1", "Laptop", Some("device-c1")),
            contact("c2", "Neu", None),
        ],
        rooms: vec![room()],
        ..ShareProfiles::default()
    }
}

#[test]
fn screen_task_viewer_launch_args() {
    // The command line.
    let view = Wrapper::try_parse_from([
        "screen",
        "view",
        "Laptop",
        "--source",
        "monitor:2",
        "--control",
        "--fullscreen",
        "--json",
    ])
    .unwrap();
    let ScreenCommand::View(args) = view.command else {
        panic!("not the view command");
    };
    assert_eq!(
        args,
        ViewArgs {
            device: "Laptop".into(),
            source: Some("monitor:2".into()),
            control: true,
            fullscreen: true,
            max_rate: None,
            codec: None,
            decoder: None,
            json: true,
        }
    );
    assert!(Wrapper::try_parse_from(["screen", "view"]).is_err());
    let grid = Wrapper::try_parse_from(["screen", "grid", "--room", "Ops"]).unwrap();
    assert!(matches!(grid.command, ScreenCommand::Grid(_)));
    assert!(matches!(
        Wrapper::try_parse_from(["screen", "grid"]).unwrap().command,
        ScreenCommand::Grid(_)
    ));

    // The target JSON of a Direct device and of a room member.
    let choices = device_choices(&profiles(), 1_760_000_000);
    let direct = select_from(&choices, "Laptop").unwrap();
    let value = target_json(&direct, Some("monitor:2"), true, true).unwrap();
    assert_eq!(
        value,
        json!({
            "relation": "direct",
            "relation_id": "c1",
            "device_id": "device-c1",
            "source": "monitor:2",
            "control": true,
            "fullscreen": true,
        })
    );
    let plain = target_json(&direct, None, false, false).unwrap();
    assert_eq!(plain["source"], serde_json::Value::Null);
    assert!(plain.get("fullscreen").is_none());
    let member = select_from(&choices, "Rack").unwrap();
    let member_value = target_json(&member, None, false, false).unwrap();
    assert_eq!(member_value["relation"], "room");
    assert_eq!(member_value["relation_id"], "room-1");
    assert_eq!(member_value["device_id"], "device-m1");

    // A source that is none, and a device that has no ID yet, are refused.
    assert!(target_json(&direct, Some("desktop"), false, false).is_err());
    let unknown = select_from(&choices, "Neu").unwrap();
    let error = target_json(&unknown, None, false, false).unwrap_err();
    assert!(error.contains("no device id"), "{error}");

    // The way to the viewer: the start parser gives the JSON back, and the
    // viewer reads the same device.
    let request = StartRequest::ScreenView {
        target_json: value.to_string(),
    };
    let args: Vec<_> = request.to_args();
    let StartRequest::ScreenView { target_json } = start_request::parse(&args).unwrap() else {
        panic!("not a screen view start");
    };
    let target = ScreenTarget::parse(&target_json).unwrap();
    assert_eq!(target.relation, Relation::Direct);
    assert!(target.control && target.fullscreen);
    assert_eq!(target.to_peer().unwrap(), direct.target);
    assert_eq!(
        ScreenTarget::parse(&member_value.to_string())
            .unwrap()
            .to_peer()
            .unwrap(),
        PeerOpenTarget::RoomDevice {
            room_id: "room-1".into(),
            device_id: "device-m1".into(),
        }
    );
    let grid = StartRequest::ScreenGrid {
        room: Some("profile-1".into()),
    };
    assert_eq!(start_request::parse(&grid.to_args()).unwrap(), grid);
}

/// B-09: `se screen view --max-rate` takes „Höchste Datenrate“ as `auto` or
/// bit/s with an SI prefix and refuses anything else before it opens.
#[test]
fn bildkette_task_rate_cli_max_rate_option() {
    use super::{parse_max_rate, MaxRate};
    assert_eq!(parse_max_rate("auto"), Ok(MaxRate::Auto));
    assert_eq!(parse_max_rate("AUTO"), Ok(MaxRate::Auto));
    assert_eq!(parse_max_rate("20M"), Ok(MaxRate::Bps(20_000_000)));
    assert_eq!(parse_max_rate("1.5G"), Ok(MaxRate::Bps(1_500_000_000)));
    assert_eq!(parse_max_rate("500k"), Ok(MaxRate::Bps(500_000)));
    assert_eq!(parse_max_rate("8000000"), Ok(MaxRate::Bps(8_000_000)));
    for wrong in ["", "0", "-5M", "fast", "M", "NaN"] {
        assert!(parse_max_rate(wrong).is_err(), "{wrong}");
    }
    let view = Wrapper::try_parse_from(["screen", "view", "Laptop", "--max-rate", "20M"]).unwrap();
    let ScreenCommand::View(args) = view.command else {
        panic!("not the view command");
    };
    assert_eq!(args.max_rate, Some(MaxRate::Bps(20_000_000)));
    assert!(Wrapper::try_parse_from(["screen", "view", "Laptop", "--max-rate", "x"]).is_err());
}

/// 4.10 (BK6): `--codec auto|h264|hevc|av1|lossless` and `--decoder
/// auto|hardware|software`; anything else is refused before the window.
#[test]
fn bildkette_task_codec_cli_codec_and_decoder_options() {
    use crate::screen::core::negotiate::{CodecSetting, DecoderPreference};
    let view = Wrapper::try_parse_from([
        "screen",
        "view",
        "Laptop",
        "--codec",
        "av1",
        "--decoder",
        "software",
    ])
    .unwrap();
    let ScreenCommand::View(args) = view.command else {
        panic!("not the view command");
    };
    assert_eq!(args.codec, Some(CodecSetting::Av1));
    assert_eq!(args.decoder, Some(DecoderPreference::Software));
    for (text, setting) in [
        ("auto", CodecSetting::Auto),
        ("h264", CodecSetting::H264),
        ("hevc", CodecSetting::Hevc),
        ("lossless", CodecSetting::Lossless),
    ] {
        assert_eq!(super::parse_codec(text), Ok(setting));
    }
    assert_eq!(
        super::parse_decoder("hardware"),
        Ok(DecoderPreference::Hardware)
    );
    assert_eq!(
        super::parse_decoder("auto"),
        Ok(DecoderPreference::Automatic)
    );
    assert!(Wrapper::try_parse_from(["screen", "view", "Laptop", "--codec", "vp9"]).is_err());
    assert!(Wrapper::try_parse_from(["screen", "view", "Laptop", "--decoder", "gpu"]).is_err());
}
