use super::endpoint_routes::NodeTransportOptions;
use super::session::relay_urls_from_signal;

const RELAY_URL_ENV: &str = "SE_SHARE_RELAY_URL";
const RELAY_ONLY_ENV: &str = "SE_SHARE_RELAY_ONLY";

pub(super) fn load(server: &str) -> NodeTransportOptions {
    let relay_override = std::env::var(RELAY_URL_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty());
    let relay_source = relay_override.as_deref().unwrap_or(server);
    let relay_urls = relay_urls_from_signal(relay_source);
    let relay_only = std::env::var(RELAY_ONLY_ENV)
        .map(|value| value.trim() == "1")
        .unwrap_or(false);
    NodeTransportOptions::new(relay_urls, relay_only)
}
