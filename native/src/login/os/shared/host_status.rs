//! Minimal public evidence. Administrative file ownership replaces no broker grant.
use serde::{Deserialize, Serialize};
use crate::login::{Policy, core::model::VERSION, os::platform};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HostEvidence {
    pub format: u32, pub state: String, pub release: String, pub policy_revision: u64,
    pub administrative_grants: usize, pub broker_pid: u32, pub process_instance: String,
    pub uptime_ms: u64, pub observed_at_ms: u64, pub valid_until_ms: u64,
    pub helper: Option<HostProcess>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HostProcess { pub pid: u32, pub instance: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HostActivity { pub state: String, pub reason: Option<String>, pub evidence: Option<HostEvidence> }
impl Default for HostActivity { fn default() -> Self { Self { state: "unknown".into(), reason: Some("no verified public broker activity evidence".into()), evidence: None } } }
fn millis() -> std::io::Result<u64> { u64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(std::io::Error::other)?.as_millis()).map_err(std::io::Error::other) }
pub(crate) fn publish(state: &str, boot: &str, policy: &Policy) -> std::io::Result<()> {
    publish_inner(state, boot, policy, None)
}
fn publish_inner(state: &str, _boot: &str, policy: &Policy, helper: Option<u32>) -> std::io::Result<()> {
    if !matches!(state, "active" | "idle" | "stopped") { return Err(std::io::Error::other("invalid public login activity state")); }
    let observed_at_ms = millis()?;
    // Boot binding comes from the live OS process instance plus monotone clock;
    // no private broker boot nonce, machine key or challenge is published.
    let record = HostEvidence { format: VERSION, state: state.into(), release: env!("CARGO_PKG_VERSION").into(),
        policy_revision: policy.revision, administrative_grants: policy.grants.len(), broker_pid: std::process::id(),
        process_instance: platform::host_process_instance(std::process::id())?,
        uptime_ms: platform::host_uptime_ms()?, observed_at_ms, valid_until_ms: observed_at_ms.saturating_add(5000),
        helper: helper.map(|pid| platform::host_process_instance(pid).map(|instance| HostProcess { pid, instance })).transpose()? };
    platform::write_host_evidence(&serde_json::to_vec(&record).map_err(std::io::Error::other)?)
}
pub(crate) fn activity() -> HostActivity {
    let result = (|| {
        let bytes = platform::read_host_evidence()?;
        let record: HostEvidence = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        let now = millis()?;
        if record.format != VERSION || !matches!(record.state.as_str(), "active" | "idle" | "stopped") || record.release.len() > 64
            || record.process_instance.is_empty() || record.process_instance.len() > 512 || record.broker_pid == 0 || record.policy_revision == 0 || record.administrative_grants > 256
            || record.valid_until_ms < record.observed_at_ms || record.valid_until_ms - record.observed_at_ms > 5000 {
            return Err(std::io::Error::other("invalid administrative broker activity evidence"));
        }
        if now < record.observed_at_ms || platform::host_uptime_ms()? < record.uptime_ms { return Err(std::io::Error::other("broker evidence belongs to a changed boot/clock context")); }
        if now >= record.valid_until_ms { return Ok(HostActivity { state: "expired".into(), reason: Some("last broker activity evidence expired; current activity is not established".into()), evidence: Some(record) }); }
        if record.state != "stopped" && platform::host_process_instance(record.broker_pid)? != record.process_instance {
            return Err(std::io::Error::other("broker process instance no longer matches its administrative evidence"));
        }
        if record.state == "active" {
            let helper = record.helper.as_ref().ok_or_else(|| std::io::Error::other("active broker status has no helper-process evidence"))?;
            if helper.pid == 0 || helper.pid == record.broker_pid || helper.instance.len() > 512 || platform::host_process_instance(helper.pid)? != helper.instance {
                return Err(std::io::Error::other("login helper process instance no longer matches its administrative evidence"));
            }
        }
        Ok(HostActivity { state: record.state.clone(), reason: None, evidence: Some(record) })
    })();
    result.unwrap_or_else(|error: std::io::Error| HostActivity { state: if error.kind() == std::io::ErrorKind::Unsupported { "unavailable" } else { "unknown" }.into(), reason: Some(error.to_string()), evidence: None })
}
pub(crate) struct SessionEvidence<'a> { boot: &'a str, policy: Policy, last: Option<(Option<u32>, u64, std::time::Instant)> }
impl<'a> SessionEvidence<'a> {
    pub fn new(boot: &'a str, policy: &Policy) -> Self { Self { boot, policy: policy.clone(), last: None } }
    pub fn update(&mut self, helper: Option<u32>, policy: &Policy) -> std::io::Result<()> {
        self.policy = policy.clone();
        if self.last.as_ref().is_some_and(|(old, revision, at)| *old == helper && *revision == policy.revision && at.elapsed() < std::time::Duration::from_secs(1)) { return Ok(()); }
        publish_inner(if helper.is_some() { "active" } else { "idle" }, self.boot, policy, helper)?;
        self.last = Some((helper, policy.revision, std::time::Instant::now())); Ok(())
    }
}
impl Drop for SessionEvidence<'_> { fn drop(&mut self) { let _ = publish("idle", self.boot, &self.policy); } }
