//! Test-only refresh scope; independent workers share one reserved endpoint.
static TEST_REFRESH_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
static TEST_REFRESH_URL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// A single test owns this override, including refreshes on worker threads.
/// `None` reserves the same scope for live tests using Google's regular URL.
pub fn test_refresh_endpoint(url: Option<&str>) -> TestRefreshEndpoint {
    if let Some(url) = url {
        assert!(url.starts_with("http://127.0.0.1:"), "OAuth fixture must be loopback");
    }
    let serial = TEST_REFRESH_SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    *TEST_REFRESH_URL.lock().unwrap_or_else(|error| error.into_inner()) = url.map(str::to_owned);
    TestRefreshEndpoint { _serial: serial }
}

pub struct TestRefreshEndpoint {
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Drop for TestRefreshEndpoint {
    fn drop(&mut self) {
        *TEST_REFRESH_URL.lock().unwrap_or_else(|error| error.into_inner()) = None;
    }
}

pub(super) fn endpoint() -> Result<Option<String>, String> {
    TEST_REFRESH_URL.lock().map(|url| url.clone()).map_err(|_| "OAuth test URL poisoned".into())
}
