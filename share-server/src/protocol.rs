use serde::{Deserialize, Serialize};

use super::tracked_direct;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DiscoveryKind {
    Direct,
    Room,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DiscoveryOperation {
    PublishDiscovery,
    UnpublishDiscovery,
    ListDiscoveries,
    StartPairing,
    PairingPacket,
    CancelPairing,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DiscoveryRejectionClass {
    Unsupported,
    InvalidRequest,
    Conflict,
    Forbidden,
    Unavailable,
    Capacity,
    RateLimited,
    Protocol,
    Internal,
}

/// Public, intentionally unlinkable metadata supplied by the publishing client.
/// Stable device/room identifiers and all key material are exchanged only inside
/// the opaque end-to-end pairing payloads.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct DiscoveryOfferRequest {
    pub(super) offer_id: String,
    pub(super) kind: DiscoveryKind,
    pub(super) display_alias: String,
    pub(super) suite: String,
    pub(super) version: u32,
    pub(super) lease_secs: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct DiscoveryAdvertisement {
    pub(super) discovery_id: String,
    pub(super) offer_id: String,
    pub(super) kind: DiscoveryKind,
    pub(super) display_alias: String,
    pub(super) suite: String,
    pub(super) version: u32,
    pub(super) expires_at: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum PairingPacketKind {
    OpaqueKe2,
    OpaqueKe3Bundle,
    PublisherBundle,
    ConnectorCommit,
    PublisherCommit,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum PairingCloseReason {
    Completed,
    Cancelled,
    TimedOut,
    OfferExpired,
    OfferWithdrawn,
    PeerDisconnected,
    TargetUnavailable,
    ProtocolError,
}

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
    /// `key_login_v1`: the answer to `hello_challenge`, an Ed25519 signature
    /// (hex) of the login digest with the key of the Hello (FC4).
    HelloAuth {
        signature: String,
    },
    PublishDirect {
        presence: PeerPresence,
        /// SHA-256 (hex) of the owner's relation access proof; watchers that
        /// logged in with a key must show the proof.
        #[serde(default)]
        access_hash: Option<String>,
    },
    UnpublishDirect {
        lookup_id: String,
    },
    WatchDirect {
        lookup_id: String,
        /// Relation access proof (hex) derived from the Direct code.
        #[serde(default)]
        access_proof: Option<String>,
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
        /// Relation access proof (hex) derived from the room secret; members
        /// only see members with the same proof (or without one).
        #[serde(default)]
        access_proof: Option<String>,
    },
    LeaveRoom {
        room_id: String,
    },
    PublishDiscovery {
        offer: DiscoveryOfferRequest,
    },
    UnpublishDiscovery {
        offer_id: String,
    },
    ListDiscoveries,
    StartPairing {
        discovery_id: String,
        exchange_id: String,
        payload: String,
    },
    PairingPacket {
        exchange_id: String,
        kind: PairingPacketKind,
        payload: String,
    },
    CancelPairing {
        exchange_id: String,
    },
    Heartbeat,
    /// `idle_keepalive_v1`: the client sleeps (or wakes) and may propose a
    /// shorter keepalive interval.
    SetIdle {
        idle: bool,
        #[serde(default)]
        keepalive_secs: Option<u32>,
    },
    /// `idle_keepalive_v1`: answer to a server `keepalive`.
    KeepaliveAck,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(super) enum Out {
    HelloOk {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        capabilities: Vec<String>,
    },
    /// `key_login_v1`: 16 random bytes (hex) the client signs.
    HelloChallenge {
        nonce: String,
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
    DiscoveryPublished {
        advertisement: DiscoveryAdvertisement,
    },
    DiscoveryList {
        advertisements: Vec<DiscoveryAdvertisement>,
    },
    PairingOpened {
        exchange_id: String,
        discovery_id: String,
    },
    PairingStarted {
        exchange_id: String,
        discovery_id: String,
        payload: String,
    },
    PairingPacket {
        exchange_id: String,
        kind: PairingPacketKind,
        payload: String,
    },
    PairingFinished {
        exchange_id: String,
        reason: PairingCloseReason,
    },
    DiscoveryRejected {
        operation: DiscoveryOperation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offer_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        discovery_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exchange_id: Option<String>,
        classification: DiscoveryRejectionClass,
        retryable: bool,
        msg: String,
    },
    Error {
        scope: String,
        msg: String,
    },
    Pong,
    /// `idle_keepalive_v1`: confirms `set_idle` with the interval in effect.
    IdleAck {
        idle: bool,
        keepalive_secs: u32,
    },
    /// `idle_keepalive_v1`: liveness probe to an idle client.
    Keepalive,
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
    fn review_task_key_login_and_access_fields_are_optional_on_the_wire() {
        // Rows of incoming lines: each line parses to the variant and fields its
        // row names. Rows of outgoing values: each value serializes to its line.
        type Row = (&'static str, &'static str, fn(&In) -> bool);
        let incoming: [Row; 7] = [
            (
                "set_idle with keepalive",
                r#"{"t":"set_idle","idle":true,"keepalive_secs":90}"#,
                |value: &In| {
                    matches!(
                        value,
                        In::SetIdle {
                            idle: true,
                            keepalive_secs: Some(90)
                        }
                    )
                },
            ),
            (
                "set_idle awake",
                r#"{"t":"set_idle","idle":false}"#,
                |value: &In| {
                    matches!(
                        value,
                        In::SetIdle {
                            idle: false,
                            keepalive_secs: None
                        }
                    )
                },
            ),
            ("keepalive ack", r#"{"t":"keepalive_ack"}"#, |value: &In| {
                matches!(value, In::KeepaliveAck)
            }),
            (
                "hello with all fields",
                r#"{"t":"hello","protocol_version":3,"device_id":"a","device_name":"Laptop","listen_port":0,"lan":["192.168.1.5"],"public_key":"pk","fingerprint":"fp"}"#,
                |value: &In| {
                    matches!(
                        value,
                        In::Hello {
                            protocol_version: 3,
                            device_id,
                            listen_port: 0,
                            ..
                        } if device_id == "a"
                    )
                },
            ),
            (
                "hello_auth signature",
                r#"{"t":"hello_auth","signature":"ab"}"#,
                |value: &In| matches!(value, In::HelloAuth { signature } if signature == "ab"),
            ),
            (
                "watch_direct without proof",
                r#"{"t":"watch_direct","lookup_id":"l"}"#,
                |value: &In| {
                    matches!(
                        value,
                        In::WatchDirect {
                            access_proof: None,
                            ..
                        }
                    )
                },
            ),
            (
                "watch_direct with proof",
                r#"{"t":"watch_direct","lookup_id":"l","access_proof":"cd"}"#,
                |value: &In| {
                    matches!(
                        value,
                        In::WatchDirect { access_proof: Some(proof), .. } if proof == "cd"
                    )
                },
            ),
        ];
        for (case, line, matches_row) in incoming {
            let value: In = serde_json::from_str(line).expect(case);
            assert!(matches_row(&value), "{case}");
        }

        let outgoing: [(&str, Out, &str); 3] = [
            (
                "idle ack",
                Out::IdleAck {
                    idle: true,
                    keepalive_secs: 180,
                },
                r#"{"t":"idle_ack","idle":true,"keepalive_secs":180}"#,
            ),
            ("keepalive", Out::Keepalive, r#"{"t":"keepalive"}"#),
            (
                "hello challenge",
                Out::HelloChallenge {
                    nonce: "00ff".into(),
                },
                r#"{"t":"hello_challenge","nonce":"00ff"}"#,
            ),
        ];
        for (case, value, line) in outgoing {
            assert_eq!(serde_json::to_string(&value).expect(case), line, "{case}");
        }
    }
}
