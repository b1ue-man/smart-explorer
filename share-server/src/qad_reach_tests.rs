use std::time::Instant;

use super::{
    family_of, judge, Family, QadReport, QadSource, Reach, Tracker, Witness, GRACE, WINDOW,
};
use crate::limits::SourceKey;
use crate::protocol::Out;

fn v4(octets: [u8; 4]) -> SourceKey {
    SourceKey::Ipv4(octets)
}

fn listening_tracker() -> Tracker {
    let mut tracker = Tracker::new();
    tracker.listening = (true, true);
    tracker
}

#[test]
fn direkt_internet_task_qad_verdict_rules() {
    assert_eq!(judge(false, true, true), Reach::Off);
    assert_eq!(judge(true, true, true), Reach::Reachable);
    assert_eq!(judge(true, false, true), Reach::Unreachable);
    assert_eq!(judge(true, false, false), Reach::Unknown);
}

#[test]
fn direkt_internet_task_qad_only_public_sources_count() {
    assert_eq!(family_of(v4([203, 0, 113, 7])), Some(Family::V4));
    for local in [
        [10, 1, 2, 3],
        [192, 168, 1, 2],
        [172, 16, 0, 1],
        [127, 0, 0, 1],
        [169, 254, 1, 1],
        [100, 64, 0, 1],
    ] {
        assert_eq!(family_of(v4(local)), None, "{local:?}");
    }
    assert_eq!(
        family_of(SourceKey::Ipv6Prefix64([
            0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 1
        ])),
        Some(Family::V6)
    );
    for local in [
        [0xfd, 0, 0, 0, 0, 0, 0, 1],
        [0xfe, 0x80, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 0, 0, 0],
    ] {
        assert_eq!(family_of(SourceKey::Ipv6Prefix64(local)), None);
    }
    assert_eq!(family_of(SourceKey::ExternallyLimitedProxy), None);
}

#[test]
fn direkt_internet_task_qad_evidence_per_family() {
    let start = Instant::now();
    let mut tracker = listening_tracker();
    tracker.witnesses.insert(
        1,
        Witness {
            source: v4([198, 51, 100, 9]),
            since: start,
        },
    );
    // Within the grace the device may still be probing.
    let early = tracker.evaluate(start);
    assert_eq!(early.ipv4, Reach::Unknown);
    // After it: no QAD connection over IPv4, the firewall drops UDP 7842.
    let later = start + GRACE;
    assert_eq!(
        tracker.evaluate(later),
        QadReport {
            ipv4: Reach::Unreachable,
            ipv6: Reach::Unknown,
        }
    );
    // A QAD connection from any public IPv4 source proves the opposite.
    tracker.qad.insert(
        v4([203, 0, 113, 50]),
        QadSource {
            held: 0,
            last: later,
        },
    );
    assert_eq!(tracker.evaluate(later).ipv4, Reach::Reachable);
    // An ended connection counts for the window, a held one as long as it lasts.
    assert_eq!(
        tracker.evaluate(later + WINDOW + GRACE).ipv4,
        Reach::Unreachable
    );
    tracker.qad.insert(
        v4([203, 0, 113, 51]),
        QadSource {
            held: 1,
            last: later,
        },
    );
    assert_eq!(
        tracker.evaluate(later + WINDOW + GRACE).ipv4,
        Reach::Reachable
    );
    // A private QAD source proves nothing about the internet.
    let mut lan = listening_tracker();
    lan.witnesses.insert(
        1,
        Witness {
            source: v4([198, 51, 100, 9]),
            since: start,
        },
    );
    lan.qad.insert(
        v4([192, 168, 1, 20]),
        QadSource {
            held: 1,
            last: start,
        },
    );
    assert_eq!(lan.evaluate(later).ipv4, Reach::Unreachable);
}

#[test]
fn direkt_internet_task_qad_status_on_the_wire() {
    let report = QadReport {
        ipv4: Reach::Reachable,
        ipv6: Reach::Off,
    };
    let hello = serde_json::to_value(Out::HelloOk {
        capabilities: vec![super::CAPABILITY.to_string()],
        qad: Some(report),
    })
    .unwrap();
    assert_eq!(
        hello["qad"],
        serde_json::json!({"ipv4": "reachable", "ipv6": "off"})
    );
    let legacy = serde_json::to_value(Out::HelloOk {
        capabilities: Vec::new(),
        qad: None,
    })
    .unwrap();
    assert_eq!(legacy, serde_json::json!({"t": "hello_ok"}));
    let status = serde_json::to_value(Out::QadStatus { qad: report }).unwrap();
    assert_eq!(status["t"], "qad_status");
}
