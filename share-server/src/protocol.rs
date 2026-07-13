use serde::{Deserialize, Serialize};

use super::tracked_direct;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct PeerPresence {
    pub(super) kind: String,
    pub(super) relation_id: String,
    pub(super) device_id: String,
    pub(super) device_name: String,
    pub(super) public_key: String,
    pub(super) fingerprint: String,
    #[serde(default)]
    pub(super) node_id: String,
    #[serde(default)]
    pub(super) relay_url: String,
    pub(super) candidates: Vec<String>,
    pub(super) expires_at: i64,
    pub(super) nonce: String,
    pub(super) proof: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(super) enum In {
    Hello {
        protocol_version: u32,
        device_id: String,
        device_name: String,
        listen_port: u16,
        #[serde(default)]
        lan: Vec<String>,
        public_key: String,
        fingerprint: String,
        #[serde(default)]
        capabilities: Vec<String>,
    },
    PublishDirect {
        presence: PeerPresence,
    },
    UnpublishDirect {
        lookup_id: String,
    },
    WatchDirect {
        lookup_id: String,
    },
    RequestDirect {
        lookup_id: String,
        presence: PeerPresence,
    },
    DirectAccessAccepted {
        lookup_id: String,
        requester_device_id: String,
        accepted: bool,
        presence: Option<PeerPresence>,
        msg: Option<String>,
    },
    SubmitDirectRequest {
        request: Box<tracked_direct::SignedDirectRequest>,
        #[serde(default)]
        legacy_presence: Option<PeerPresence>,
    },
    SubmitDirectRequestReceipt {
        receipt: tracked_direct::SignedDirectRequestReceipt,
    },
    SubmitDirectDecision {
        decision: tracked_direct::SignedDirectDecision,
    },
    SubmitDirectDecisionReceipt {
        receipt: tracked_direct::SignedDirectDecisionReceipt,
    },
    UnwatchDirect {
        lookup_id: String,
    },
    JoinRoom {
        room_id: String,
        presence: PeerPresence,
    },
    LeaveRoom {
        room_id: String,
    },
    Heartbeat,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(super) enum Out {
    HelloOk {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        capabilities: Vec<String>,
    },
    DirectAvailable {
        lookup_id: String,
        presence: PeerPresence,
    },
    DirectOffline {
        lookup_id: String,
    },
    DirectAccessRequest {
        lookup_id: String,
        presence: PeerPresence,
    },
    DirectAccessAccepted {
        lookup_id: String,
        requester_device_id: String,
        accepted: bool,
        presence: Option<PeerPresence>,
        msg: Option<String>,
    },
    DirectRequest {
        request: tracked_direct::SignedDirectRequest,
    },
    DirectRequestReceipt {
        receipt: tracked_direct::SignedDirectRequestReceipt,
    },
    DirectDecision {
        decision: tracked_direct::SignedDirectDecision,
    },
    DirectDecisionReceipt {
        receipt: tracked_direct::SignedDirectDecisionReceipt,
    },
    DirectRouteAck {
        request_id: String,
        route: tracked_direct::DirectRoute,
        outcome: tracked_direct::DirectRouteOutcome,
    },
    RoomRoster {
        room_id: String,
        members: Vec<PeerPresence>,
    },
    RoomJoined {
        room_id: String,
        presence: PeerPresence,
    },
    RoomLeft {
        room_id: String,
        device_id: String,
    },
    Error {
        scope: String,
        msg: String,
    },
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_messages_serialize_with_stable_tags() {
        let offline = Out::DirectOffline {
            lookup_id: "x".into(),
        };
        assert_eq!(
            serde_json::to_string(&offline).unwrap(),
            r#"{"t":"direct_offline","lookup_id":"x"}"#
        );
        let roster = Out::RoomRoster {
            room_id: "r".into(),
            members: vec![],
        };
        assert_eq!(
            serde_json::to_string(&roster).unwrap(),
            r#"{"t":"room_roster","room_id":"r","members":[]}"#
        );
    }

    #[test]
    fn hello_parses() {
        let hello: In = serde_json::from_str(
            r#"{"t":"hello","protocol_version":3,"device_id":"a","device_name":"Laptop","listen_port":0,"lan":["192.168.1.5"],"public_key":"pk","fingerprint":"fp"}"#,
        )
        .unwrap();
        let In::Hello {
            protocol_version,
            device_id,
            listen_port,
            ..
        } = hello
        else {
            panic!("not hello");
        };
        assert_eq!(protocol_version, 3);
        assert_eq!(device_id, "a");
        assert_eq!(listen_port, 0);
    }

    #[test]
    fn presence_roundtrips() {
        let presence = PeerPresence {
            kind: "room".into(),
            relation_id: "r".into(),
            device_id: "d".into(),
            device_name: "Device".into(),
            public_key: "pk".into(),
            fingerprint: "fp".into(),
            node_id: "node".into(),
            relay_url: "http://127.0.0.1:51821".into(),
            candidates: vec!["127.0.0.1:1".into()],
            expires_at: 99,
            nonce: "n".into(),
            proof: "proof".into(),
        };
        let serialized = serde_json::to_string(&presence).unwrap();
        let parsed: PeerPresence = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed.kind, "room");
        assert_eq!(parsed.relation_id, "r");
        assert_eq!(parsed.device_id, "d");
    }
}
