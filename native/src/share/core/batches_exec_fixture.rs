//! Real ShareExecInput construction for the remote owner/cancellation fixtures.
use super::*;
use crate::share::types::ShareAuthState;
use tokio::sync::mpsc;

pub(crate) struct ExecInputFixture {
    pub(crate) input: ShareExecInput,
    pub(crate) data: mpsc::Receiver<ExecClientInput>,
    pub(crate) cancel: watch::Receiver<bool>,
    node: Arc<ShareIrohNode>,
    _relay: std::net::TcpListener,
}

impl Drop for ExecInputFixture {
    fn drop(&mut self) { self.node.stop_sharing().unwrap(); }
}

pub(super) fn input() -> ExecInputFixture {
    // Reserve the derived relay address locally. No public relay or account
    // participates, and no global environment override is modified by a test.
    let relay = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = relay.local_addr().unwrap().port();
    assert!(port > 1);
    let server = format!("tcp://127.0.0.1:{}", port - 1);
    let options = crate::share::transport_options::load(&server);
    assert!(!options.relay_only, "suite environment must not force relay-only transport");
    assert_eq!(options.relay_urls.iter().map(|url| url.to_string().trim_end_matches('/').to_owned())
        .collect::<Vec<_>>(), vec![format!("http://127.0.0.1:{port}")],
        "suite environment must not override the isolated fixture relay");
    let secret = iroh::SecretKey::from_bytes(&[61; 32]);
    let key = secret.public().to_string();
    let identity = crate::share::identity::ShareIdentity {
        device_id: "exec-owner-fixture".into(), device_name: "exec-owner-fixture".into(),
        direct_lookup_id: "exec-owner-fixture".into(), public_key: key.clone(),
        fingerprint: crate::share::core::public_fingerprint(key.as_bytes()), node_id: key,
        iroh_secret: secret, direct_secret: [61; 32],
    };
    let auth = Arc::new(Mutex::new(ShareAuthState {
        identity: identity.clone(), direct_secret: vec![61; 32], default_direct_exports: Default::default(),
        direct_contacts: Vec::new(), direct_grants: Vec::new(), rooms: Vec::new(),
        direct_requests: Vec::new(), direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(), direct_online: false, authorization_epoch: 1,
    }));
    let (events, _event_rx) = crossbeam_channel::unbounded();
    let node = ShareIrohNode::start(&server, &identity, auth, events).unwrap();
    let (data_tx, data) = mpsc::channel(1);
    let (cancel_tx, cancel) = watch::channel(false);
    let sender = crate::share::exec_client::batches_fixture_sender(data_tx, cancel_tx);
    ExecInputFixture { input: ShareExecInput { node: node.clone(), sender },
        data, cancel, node, _relay: relay }
}
