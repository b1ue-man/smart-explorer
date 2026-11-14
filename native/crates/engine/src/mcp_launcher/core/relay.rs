//! Bookkeeping of the launcher relay between an agent host and the runtime it
//! starts. The relay keeps the host's stdio connection open while a runtime
//! ends for an update and a runtime of the confirmed generation takes over.
//!
//! The relay reads newline-delimited JSON-RPC (MCP stdio transport) only far
//! enough to know: which host requests still wait for an answer, which
//! runtime requests the host answers, and the host's `initialize` handshake.
//! Every other byte passes through unchanged.
//!
//! Rules: a request that was delivered to an ended runtime gets an error
//! answer and is never sent again (its effect is unknown). A request the host
//! sends while no runtime runs waits and goes to the next runtime. The next
//! runtime receives the host's recorded `initialize` (under an own id whose
//! answer the relay consumes) and `notifications/initialized`; a host on the
//! stateless lifecycle (no `initialize`) needs no replay. Afterwards the host
//! receives `notifications/tools/list_changed`, if an answer it received
//! declared `capabilities.tools.listChanged`. A request the host cancels while
//! it still waits for a runtime is withdrawn and reaches none.
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// JSON-RPC code of the answer to a request whose runtime ended before it
/// answered (implementation-defined server error range).
pub const RUNTIME_ENDED_CODE: i64 = -32001;
const REPLAY_ID_PREFIX: &str = "se-launcher-replay-";

#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Pass the line on unchanged.
    Forward,
    /// Consume the line (answer to the relay's own replay, or an answer of
    /// the host to a request of an ended runtime).
    Drop,
}

#[derive(Default)]
pub struct RelayBook {
    /// Host requests delivered to the current runtime and not yet answered.
    pending: BTreeSet<String>,
    /// Requests the current runtime sent to the host and not yet answered.
    runtime_requests: BTreeSet<String>,
    initialize: Option<Value>,
    initialized: Option<Vec<u8>>,
    replay_id: Option<String>,
    /// The host has sent at least one request (the session exists).
    started: bool,
    /// An answer the host received declared `capabilities.tools.listChanged`
    /// (the `initialize` result): only then it expects the notification.
    list_changed: bool,
}

fn parse(line: &[u8]) -> Option<Vec<Value>> {
    match serde_json::from_slice::<Value>(line).ok()? {
        Value::Array(batch) => Some(batch),
        message => Some(vec![message]),
    }
}
fn key(id: &Value) -> String {
    id.to_string()
}
fn is_request(message: &Value) -> bool {
    message.get("method").is_some() && message.get("id").is_some_and(|id| !id.is_null())
}
fn is_answer(message: &Value) -> bool {
    message.get("method").is_none()
        && message.get("id").is_some()
        && (message.get("result").is_some() || message.get("error").is_some())
}
fn line(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

impl RelayBook {
    /// A line from the host about to be delivered to the current runtime.
    pub fn host_to_runtime(&mut self, bytes: &[u8]) -> Route {
        let Some(messages) = parse(bytes) else {
            return Route::Forward;
        };
        let mut route = Route::Forward;
        for message in &messages {
            if is_request(message) {
                self.started = true;
                if message["method"] == "initialize" {
                    self.initialize = Some(message.clone());
                }
                self.pending.insert(key(&message["id"]));
            } else if message["method"] == "notifications/initialized" {
                self.initialized = Some(bytes.to_vec());
            } else if is_answer(message)
                && messages.len() == 1
                && !self.runtime_requests.remove(&key(&message["id"]))
            {
                // Answers a request of a runtime that has ended.
                route = Route::Drop;
            }
        }
        route
    }

    /// A line from the current runtime about to be delivered to the host.
    pub fn runtime_to_host(&mut self, bytes: &[u8]) -> Route {
        let Some(messages) = parse(bytes) else {
            return Route::Forward;
        };
        for message in &messages {
            if is_answer(message) {
                let id = key(&message["id"]);
                if messages.len() == 1
                    && self
                        .replay_id
                        .as_ref()
                        .is_some_and(|replay| key(&json!(replay)) == id)
                {
                    self.replay_id = None;
                    return Route::Drop;
                }
                if message["result"]["capabilities"]["tools"]["listChanged"] == true {
                    self.list_changed = true;
                }
                self.pending.remove(&id);
            } else if is_request(message) {
                self.runtime_requests.insert(key(&message["id"]));
            }
        }
        Route::Forward
    }

    /// The runtime ended: one error answer for each request it had not
    /// answered. Requests of the ended runtime to the host are forgotten.
    pub fn runtime_ended(&mut self, reason: &str) -> Vec<Vec<u8>> {
        self.runtime_requests.clear();
        self.replay_id = None;
        std::mem::take(&mut self.pending)
            .into_iter()
            .filter_map(|id| serde_json::from_str::<Value>(&id).ok())
            .map(|id| error_answer(&id, reason))
            .collect()
    }

    /// Lines the next runtime receives before any queued host line.
    pub fn replay(&mut self, generation: u64) -> Vec<Vec<u8>> {
        let Some(initialize) = &self.initialize else {
            return Vec::new();
        };
        let id = format!("{REPLAY_ID_PREFIX}{generation}");
        let mut request = initialize.clone();
        request["id"] = json!(id);
        self.replay_id = Some(id);
        let mut lines = vec![line(&request)];
        if let Some(initialized) = &self.initialized {
            let mut initialized = initialized.clone();
            if !initialized.ends_with(b"\n") {
                initialized.push(b'\n');
            }
            lines.push(initialized);
        }
        lines
    }

    /// Whether the host has sent a request on this connection.
    pub fn session_started(&self) -> bool {
        self.started
    }

    /// Whether the host was told that the tool list may change: only then
    /// `notifications/tools/list_changed` is sent to it (MCP, tools capability).
    pub fn announces_tool_changes(&self) -> bool {
        self.list_changed
    }

    /// The key of the request a host line withdraws (`notifications/cancelled`).
    pub fn cancelled_request(bytes: &[u8]) -> Option<String> {
        let message: Value = serde_json::from_slice(bytes).ok()?;
        if message["method"] != "notifications/cancelled" || message.get("id").is_some() {
            return None;
        }
        message["params"].get("requestId").map(key)
    }

    /// Whether the line is one request with the key `wanted`.
    pub fn is_request_with_key(bytes: &[u8], wanted: &str) -> bool {
        serde_json::from_slice::<Value>(bytes)
            .is_ok_and(|message| is_request(&message) && key(&message["id"]) == wanted)
    }

    /// A `ping` the relay answers itself while no runtime runs.
    pub fn local_ping_answer(bytes: &[u8]) -> Option<Vec<u8>> {
        let message: Value = serde_json::from_slice(bytes).ok()?;
        (message["method"] == "ping" && is_request(&message))
            .then(|| line(&json!({"jsonrpc":"2.0","id":message["id"],"result":{}})))
    }

    /// The error answer for one host request in `bytes`, if it is one.
    pub fn refusal(bytes: &[u8], reason: &str) -> Vec<Vec<u8>> {
        parse(bytes)
            .unwrap_or_default()
            .iter()
            .filter(|message| is_request(message))
            .map(|message| error_answer(&message["id"], reason))
            .collect()
    }
}

pub fn tools_list_changed() -> Vec<u8> {
    line(&json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}))
}

fn error_answer(id: &Value, reason: &str) -> Vec<u8> {
    line(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": RUNTIME_ENDED_CODE, "message": reason},
    }))
}

#[cfg(test)]
#[path = "plugin_task_relay_tests.rs"]
mod plugin_task_relay_tests;
