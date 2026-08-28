use serde::Deserialize;

use super::{In, Out};

const WIRE_FIXTURE: &str = include_str!("../../testdata/share-discovery-wire-v1.jsonl");

#[derive(Deserialize)]
struct FixtureLine {
    direction: String,
    message: serde_json::Value,
}

#[test]
fn share_remote_task_server_and_native_discovery_wire_fixture() {
    assert!(WIRE_FIXTURE.len() < 16 * 1024);
    let mut client_messages = 0;
    let mut server_messages = 0;
    for (index, line) in WIRE_FIXTURE.lines().enumerate() {
        assert!(!line.is_empty(), "blank fixture line {}", index + 1);
        assert!(line.len() < 2 * 1024, "oversized fixture line {}", index + 1);
        let fixture: FixtureLine = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("fixture line {}: {error}", index + 1));
        match fixture.direction.as_str() {
            "client" => {
                let message: In = serde_json::from_value(fixture.message)
                    .unwrap_or_else(|error| panic!("client fixture line {}: {error}", index + 1));
                assert!(matches!(
                    message,
                    In::PublishDiscovery { .. }
                        | In::UnpublishDiscovery { .. }
                        | In::ListDiscoveries
                        | In::StartPairing { .. }
                        | In::PairingPacket { .. }
                        | In::CancelPairing { .. }
                ));
                client_messages += 1;
            }
            "server" => {
                let message: Out = serde_json::from_value(fixture.message.clone())
                    .unwrap_or_else(|error| panic!("server fixture line {}: {error}", index + 1));
                assert!(matches!(
                    &message,
                    Out::DiscoveryPublished { .. }
                        | Out::DiscoveryList { .. }
                        | Out::PairingOpened { .. }
                        | Out::PairingStarted { .. }
                        | Out::PairingPacket { .. }
                        | Out::PairingFinished { .. }
                        | Out::DiscoveryRejected { .. }
                ));
                assert_eq!(serde_json::to_value(&message).unwrap(), fixture.message);
                server_messages += 1;
            }
            other => panic!("unknown fixture direction {other}"),
        }
    }
    assert_eq!((client_messages, server_messages), (6, 7));
}
