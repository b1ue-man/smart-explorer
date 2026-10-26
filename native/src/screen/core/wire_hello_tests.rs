use super::*;
use crate::screen::core::negotiate::H264Profile;
use crate::share::relation_proof::EXEC_TRANSCRIPT_PREFIX;

fn server_hello() -> ScreenServerHello {
    ScreenServerHello::new(
        "c".repeat(32),
        "host-device".into(),
        "host-key".into(),
        "host-fingerprint".into(),
        "host-node".into(),
    )
}

fn client_hello() -> ScreenClientHello {
    ScreenClientHello {
        family_version: None,
        icon_version: None,
        metadata_only: false,
        frame_capabilities: None,
        protocol_version: SCREEN_PROTOCOL_VERSION,
        relation_kind: "direct".into(),
        relation_id: "lookup".into(),
        device_id: "viewer".into(),
        device_name: "Laptop".into(),
        public_key: "viewer-key".into(),
        fingerprint: "viewer-fingerprint".into(),
        node_id: "viewer-node".into(),
        client_nonce: "n".repeat(32),
        proof: "proof".into(),
        mode: SessionMode::Control,
        source: Some(SourceId::Window("0x2a".into())),
        decoders: vec![
            CodecOffer::h264(H264Profile::High, true),
            CodecOffer::tiles(),
        ],
        display_size: Some(FrameSize::new(2560, 1440)),
        max_fps: 60,
    }
}

#[test]
fn screen_task_hello_maps_onto_relation_proof() {
    let (server, client) = (server_hello(), client_hello());
    let (relation_server, relation_client) = (server.relation(), client.relation());
    assert_eq!(relation_server.transcript_prefix, SCREEN_TRANSCRIPT_PREFIX);
    assert_eq!(relation_server.capability, SCREEN_CAPABILITY);
    assert_eq!(relation_client.capability, SCREEN_CAPABILITY);
    assert_eq!(relation_server.server_node_id, server.server_node_id);
    assert_eq!(relation_client.node_id, client.node_id);
    assert_eq!(relation_client.proof, client.proof);
    let transcript = screen_transcript(&server, &client);
    assert!(transcript.starts_with("smart-explorer/share-screen/v1|cccc"));
    assert!(transcript.ends_with("|host-node|client-to-server|screen"));
    // The proof, the mode and what the viewer asks for are not proven.
    let mut other = client.clone();
    other.proof = "other".into();
    other.mode = SessionMode::Preview;
    other.decoders.clear();
    assert_eq!(screen_transcript(&server, &other), transcript);
    // An Exec transcript over the same identities differs.
    let mut exec = relation_server.clone();
    exec.transcript_prefix = EXEC_TRANSCRIPT_PREFIX.into();
    assert_ne!(relation_transcript(&exec, &relation_client), transcript);
    // Mode rules (C6, C7) and checks beyond the proof.
    assert_eq!(
        SessionMode::Control.capabilities(),
        &[Capability::ScreenView, Capability::ScreenControl]
    );
    assert_eq!(
        SessionMode::Preview.capabilities(),
        &[Capability::ScreenView]
    );
    assert!(SessionMode::View.may_ask() && SessionMode::Control.may_ask());
    assert!(!SessionMode::Probe.may_ask() && !SessionMode::Preview.may_ask());
    for broken in [
        ScreenClientHello {
            family_version: None,
            protocol_version: 2,
            ..client.clone()
        },
        ScreenClientHello {
            family_version: None,
            max_fps: 0,
            ..client.clone()
        },
        ScreenClientHello {
            family_version: None,
            display_size: Some(FrameSize::new(0, 1080)),
            ..client.clone()
        },
    ] {
        assert!(broken.validate().is_err());
    }
    assert!(supports_version(SCREEN_PROTOCOL_VERSION) && !supports_version(2));
    let refused = ScreenHelloError::new(ScreenErrorCode::IndicatorUnavailable);
    assert_eq!(refused.code, "indicator_unavailable");
    assert_eq!(
        refused.known_code(),
        Some(ScreenErrorCode::IndicatorUnavailable)
    );
    let future = ScreenHelloError {
        code: "future_code".into(),
    };
    assert_eq!(future.known_code(), None);
    for code in ScreenErrorCode::ALL {
        assert_eq!(ScreenErrorCode::from_key(code.key()), Some(code));
    }
}

#[test]
fn batches_task_b6_hello_class_is_backward_compatible_before_client_offer() {
    use crate::screen::core::frame_budget::{FrameCapabilities,FrameContract,MAX_WORKING_BYTES};
    let modern=server_hello();
    let mut old_json=serde_json::to_value(&modern).unwrap();
    old_json.as_object_mut().unwrap().remove("frame_memory_class");
    let old:ScreenServerHello=serde_json::from_value(old_json).unwrap();
    assert_eq!(old.frame_memory_class,None);
    let caps=FrameCapabilities::admitted(FrameSize::new(16384,16384),768*1024*1024).unwrap();
    let modern_offer=caps.for_server(modern.frame_memory_class).unwrap();
    let old_offer=caps.for_server(old.frame_memory_class).unwrap();
    assert_eq!(old_offer.frames.working_bytes,MAX_WORKING_BYTES);
    assert!(modern_offer.frames.check(FrameSize::new(3840,2160)).is_ok());
    let mut hello=client_hello();hello.frame_capabilities=Some(old_offer);hello.validate().unwrap();
    // This is the old implementation's additional validator ceiling.
    assert!(hello.frame_capabilities.unwrap().frames.working_bytes<=512*1024*1024);
    let accepted=FrameContract::negotiate(Some(modern_offer),Some(old_offer)).unwrap().unwrap();
    assert_eq!(accepted.limits.working_bytes,MAX_WORKING_BYTES);
    assert_ne!(accepted.limits.fit(FrameSize::new(3840,2160)),FrameSize::new(3840,2160));
    assert_eq!(screen_transcript(&old,&hello),screen_transcript(&modern,&hello));
    // An old serde reader ignores the optional new JSON marker.
    #[derive(Deserialize)]
    struct OldServer {protocol_version:u16,challenge:String}
    let legacy:OldServer=serde_json::from_slice(&serde_json::to_vec(&modern).unwrap()).unwrap();
    assert_eq!(legacy.protocol_version,modern.protocol_version);assert_eq!(legacy.challenge,modern.challenge);
}
