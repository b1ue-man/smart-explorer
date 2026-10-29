//! Credentialstore and OAuth fixture scope; secrets stay in the normal store.
use super::sync_reliability_task_fixture::DriveFixture;
use super::task_http::{Answer, Request};
use serde_json::json;

/// Preserve the regular test profile's credentials around OAuth fixtures.
/// The scoped URL serializes fake/live refreshes, including Rayon workers.
/// Each fixture starts from an empty Drive auth session: the session keeps
/// the confirmed account across disconnects and token imports
/// (cloud/os/coordinator.rs `disconnect`, `store_import`, since b27d0549,
/// v0.5.185), so a session left by an earlier test of the same profile
/// would bind this fixture's fake account to another one.
pub(super) struct CloudCredentials {
    previous: crate::cloud::ClientConfig,
    session: Option<crate::cloud::TestSessionSnapshot>,
    _endpoint: crate::cloud::TestRefreshEndpoint,
}

impl CloudCredentials {
    pub(super) fn install(
        config: crate::cloud::ClientConfig,
        refresh: &str,
        url: Option<&str>,
    ) -> Self {
        let endpoint = crate::cloud::test_refresh_endpoint(url);
        let previous = crate::cloud::load_config_checked(crate::cloud::Provider::GDrive)
            .unwrap_or_else(|_| {
                panic!("C10/OAuth fixture cannot read isolated cloud configuration")
            });
        let session = crate::cloud::test_session_snapshot(crate::cloud::Provider::GDrive)
            .unwrap_or_else(|_| panic!("C10/OAuth fixture cannot read isolated credential store"));
        let guard = Self {
            previous,
            session: Some(session),
            _endpoint: endpoint,
        };
        crate::cloud::test_reset_session(crate::cloud::Provider::GDrive)
            .unwrap_or_else(|_| panic!("C10/OAuth fixture cannot reset isolated credential store"));
        crate::cloud::save_config(crate::cloud::Provider::GDrive, &config).unwrap_or_else(|_| {
            panic!("C10/OAuth fixture cannot save isolated cloud configuration")
        });
        crate::cloud::store_refresh_token(crate::cloud::Provider::GDrive, refresh).unwrap_or_else(
            |_| panic!("C10/OAuth fixture cannot access isolated credential store"),
        );
        guard
    }
}

impl Drop for CloudCredentials {
    fn drop(&mut self) {
        let config = crate::cloud::save_config(crate::cloud::Provider::GDrive, &self.previous);
        // After the configuration: a client change records itself in the
        // session, which the snapshot then replaces as a whole.
        let token = match self.session.take() {
            Some(session) => {
                crate::cloud::test_restore_session(crate::cloud::Provider::GDrive, session)
            }
            None => Ok(()),
        };
        if !std::thread::panicking() {
            assert!(
                config.is_ok() && token.is_ok(),
                "OAuth fixture credential cleanup failed"
            );
        }
    }
}

pub(super) fn token_answer(request: &Request) -> Option<Answer> {
    (request.path() == "/oauth/token").then(|| {
        assert_eq!(request.method, "POST");
        assert!(String::from_utf8_lossy(&request.body).contains("grant_type=refresh_token"));
        Answer::json(
            json!({"access_token": "renewed-fixture-token", "expires_in": 3600,
            "token_type": "Bearer", "refresh_token": "rotated-fixture-refresh"}),
        )
    })
}

pub(super) fn oauth_credentials(f: &DriveFixture) -> CloudCredentials {
    CloudCredentials::install(
        crate::cloud::ClientConfig {
            client_id: "fixture-client".into(),
            client_secret: String::new(),
        },
        "fixture-refresh",
        Some(&f.server.url("/oauth/token")),
    )
}

pub(super) fn oauth_posts(f: &DriveFixture) -> usize {
    f.server
        .requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path() == "/oauth/token")
        .count()
}
