//! The relay's message bookkeeping across a runtime change (agent plugin
//! repair of 2026-11-14, cause U2): open requests of an ended runtime are
//! answered once and never resent, the next runtime receives the host's
//! handshake, and only the relay consumes the answer to it.
use super::{tools_list_changed, RelayBook, Route, RUNTIME_ENDED_CODE};
use serde_json::{json, Value};

fn bytes(value: Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(&value).unwrap();
    line.push(b'\n');
    line
}
fn parsed(line: &[u8]) -> Value {
    serde_json::from_slice(line).unwrap()
}
fn initialize(id: i64) -> Vec<u8> {
    bytes(
        json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"host","version":"1"}}}),
    )
}
fn initialized() -> Vec<u8> {
    bytes(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
}
fn call(id: i64) -> Vec<u8> {
    bytes(
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"status","arguments":{}}}),
    )
}
fn answer(id: Value) -> Vec<u8> {
    bytes(json!({"jsonrpc":"2.0","id":id,"result":{}}))
}

#[test]
fn plugin_task_relay_answers_each_open_request_of_an_ended_runtime_once() {
    let mut book = RelayBook::default();
    assert_eq!(book.host_to_runtime(&initialize(0)), Route::Forward);
    assert_eq!(book.host_to_runtime(&initialized()), Route::Forward);
    assert_eq!(book.runtime_to_host(&answer(json!(0))), Route::Forward);
    assert_eq!(book.host_to_runtime(&call(1)), Route::Forward);
    assert_eq!(book.host_to_runtime(&call(2)), Route::Forward);
    assert_eq!(book.runtime_to_host(&answer(json!(1))), Route::Forward);
    let refused = book.runtime_ended("ended");
    assert_eq!(refused.len(), 1, "only the unanswered call 2 is open");
    let refusal = parsed(&refused[0]);
    assert_eq!(refusal["id"], json!(2));
    assert_eq!(refusal["error"]["code"], json!(RUNTIME_ENDED_CODE));
    assert_eq!(refusal["error"]["message"], json!("ended"));
    assert!(refused[0].ends_with(b"\n"));
    assert!(
        book.runtime_ended("again").is_empty(),
        "an answered request is never answered twice"
    );
}

#[test]
fn plugin_task_relay_replays_the_handshake_and_consumes_only_its_answer() {
    let mut book = RelayBook::default();
    book.host_to_runtime(&initialize(0));
    book.host_to_runtime(&initialized());
    book.runtime_to_host(&answer(json!(0)));
    book.runtime_ended("ended");
    let replay = book.replay(2);
    assert_eq!(replay.len(), 2);
    let request = parsed(&replay[0]);
    assert_eq!(request["method"], json!("initialize"));
    assert_eq!(request["id"], json!("se-launcher-replay-2"));
    assert_eq!(request["params"]["protocolVersion"], json!("2025-06-18"));
    assert_eq!(
        parsed(&replay[1])["method"],
        json!("notifications/initialized")
    );
    // A host answer with the same id as a host request is not the replay's.
    assert_eq!(book.runtime_to_host(&answer(json!(0))), Route::Forward);
    assert_eq!(
        book.runtime_to_host(&answer(json!("se-launcher-replay-2"))),
        Route::Drop
    );
    assert_eq!(
        book.runtime_to_host(&answer(json!("se-launcher-replay-2"))),
        Route::Forward,
        "only the one answer to the replay is consumed"
    );
    assert!(book.session_started());
}

#[test]
fn plugin_task_relay_announces_tool_changes_only_after_the_capability_was_declared() {
    // A runtime that declares no `tools.listChanged` (a release before the
    // relay, reached after a rollback): the host is never sent the notification.
    let mut silent = RelayBook::default();
    silent.host_to_runtime(&initialize(0));
    let plain = bytes(json!({"jsonrpc":"2.0","id":0,"result":{
        "protocolVersion":"2025-06-18","capabilities":{"tools":{}}}}));
    assert_eq!(silent.runtime_to_host(&plain), Route::Forward);
    assert!(silent.session_started());
    assert!(!silent.announces_tool_changes());

    let mut declared = RelayBook::default();
    declared.host_to_runtime(&initialize(0));
    assert!(!declared.announces_tool_changes());
    let capable = bytes(json!({"jsonrpc":"2.0","id":0,"result":{
        "protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":true}}}}));
    assert_eq!(declared.runtime_to_host(&capable), Route::Forward);
    assert!(declared.announces_tool_changes());
    // The declaration holds across the runtime change; the consumed answer
    // to the replay does not count as one the host received.
    declared.runtime_ended("ended");
    assert!(declared.announces_tool_changes());
    let mut replayed = RelayBook::default();
    replayed.host_to_runtime(&initialize(0));
    replayed.runtime_to_host(&plain);
    replayed.runtime_ended("ended");
    replayed.replay(2);
    let answer_to_replay = bytes(
        json!({"jsonrpc":"2.0","id":"se-launcher-replay-2","result":{
        "capabilities":{"tools":{"listChanged":true}}}}),
    );
    assert_eq!(replayed.runtime_to_host(&answer_to_replay), Route::Drop);
    assert!(!replayed.announces_tool_changes());
}

#[test]
fn plugin_task_relay_names_the_request_a_cancellation_withdraws() {
    let cancel = |id: Value| {
        bytes(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}}))
    };
    let withdrawn = RelayBook::cancelled_request(&cancel(json!(4))).expect("the cancelled request");
    assert!(RelayBook::is_request_with_key(&call(4), &withdrawn));
    assert!(!RelayBook::is_request_with_key(&call(5), &withdrawn));
    // The id 4 as a number is another request than the id "4" as a string.
    let text = RelayBook::cancelled_request(&cancel(json!("4"))).expect("the cancelled request");
    assert!(!RelayBook::is_request_with_key(&call(4), &text));
    // Only a cancellation names a request to withdraw; a notification is no request.
    assert!(RelayBook::cancelled_request(&call(4)).is_none());
    assert!(RelayBook::cancelled_request(&initialized()).is_none());
    assert!(RelayBook::cancelled_request(b"not json\n").is_none());
    assert!(!RelayBook::is_request_with_key(&initialized(), &withdrawn));
}

#[test]
fn plugin_task_relay_stateless_host_needs_no_replay() {
    let mut book = RelayBook::default();
    assert!(!book.session_started());
    let listed = bytes(
        json!({"jsonrpc":"2.0","id":5,"method":"tools/list","params":{"_meta":{
        "io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}),
    );
    assert_eq!(book.host_to_runtime(&listed), Route::Forward);
    assert!(book.session_started());
    assert!(book.replay(2).is_empty());
    assert_eq!(book.runtime_ended("ended").len(), 1);
}

#[test]
fn plugin_task_relay_drops_host_answers_to_an_ended_runtime() {
    let mut book = RelayBook::default();
    let elicit = bytes(json!({"jsonrpc":"2.0","id":7,"method":"elicitation/create","params":{}}));
    assert_eq!(book.runtime_to_host(&elicit), Route::Forward);
    assert_eq!(book.host_to_runtime(&answer(json!(7))), Route::Forward);
    assert_eq!(book.runtime_to_host(&elicit), Route::Forward);
    book.runtime_ended("ended");
    assert_eq!(book.host_to_runtime(&answer(json!(7))), Route::Drop);
}

#[test]
fn plugin_task_relay_passes_lines_it_cannot_read_unchanged() {
    let mut book = RelayBook::default();
    assert_eq!(book.host_to_runtime(b"not json\n"), Route::Forward);
    assert_eq!(book.runtime_to_host(b"\n"), Route::Forward);
    let notification =
        bytes(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}));
    assert_eq!(book.host_to_runtime(&notification), Route::Forward);
    assert!(book.runtime_ended("ended").is_empty());
}

#[test]
fn plugin_task_relay_answers_ping_and_refuses_queued_requests_without_runtime() {
    let ping = bytes(json!({"jsonrpc":"2.0","id":"p","method":"ping"}));
    let pong = parsed(&RelayBook::local_ping_answer(&ping).unwrap());
    assert_eq!(pong["id"], json!("p"));
    assert_eq!(pong["result"], json!({}));
    assert!(RelayBook::local_ping_answer(&call(1)).is_none());
    assert!(RelayBook::local_ping_answer(&initialized()).is_none());
    let refused = RelayBook::refusal(&call(9), "gone");
    assert_eq!(refused.len(), 1);
    assert_eq!(parsed(&refused[0])["id"], json!(9));
    assert!(RelayBook::refusal(&initialized(), "gone").is_empty());
    let changed = parsed(&tools_list_changed());
    assert_eq!(changed["method"], json!("notifications/tools/list_changed"));
    assert!(changed.get("id").is_none());
}
