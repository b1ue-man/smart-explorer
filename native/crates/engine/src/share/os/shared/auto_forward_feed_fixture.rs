//! Test support: the feed of one device as `apply` receives it from
//! `peer_feed`, on a Share service of this test process (Desktop „Öffnen“
//! and „Schließen“, E27, E50, M26; `dienste_oeffnen_task_tests.rs`). The
//! device's mappings end when the feed is dropped.
use crate::share::loopback_services::{BindScope, LoopbackService, OwnerClass};
use crate::share::{PeerOpenTarget, ShareService};

pub(crate) struct Feed {
    service: ShareService,
    device_id: String,
    public_key: String,
}

impl Feed {
    pub(crate) fn new(device_id: &str, public_key: &str) -> Self {
        Self {
            service: crate::share::service_tests::test_service(),
            device_id: device_id.into(),
            public_key: public_key.into(),
        }
    }

    /// The device's feed now lists exactly `ports` (loopback services of a
    /// person's account), like one feed update.
    pub(crate) fn update(&self, ports: &[u16]) -> Result<(), String> {
        let services: Vec<LoopbackService> = ports
            .iter()
            .map(|&port| LoopbackService {
                port,
                scope: BindScope::LoopbackOnly,
                owner: OwnerClass::NormalUser {
                    uid_or_sid: "1000".into(),
                },
                process: Some(format!("dienst-{port}")),
                pid: None,
            })
            .collect();
        let target = PeerOpenTarget::Direct {
            contact_id: format!("feed-{}", self.device_id),
        };
        super::apply(
            &self.service,
            &self.device_id,
            &self.public_key,
            &target,
            &services,
        )
    }

    /// Whether the service on `port` has a listener on this device now.
    pub(crate) fn listening(&self, port: u16) -> bool {
        super::views(&self.device_id, &self.public_key)
            .iter()
            .any(|view| view.host_port == port && view.address.is_some())
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        super::remove(&self.device_id, &self.public_key);
    }
}
