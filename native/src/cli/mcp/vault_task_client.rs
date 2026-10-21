//! A small MCP client for the stage tests of `se mcp` (stage `vault_mcp`,
//! vault plan VLT-M9): it starts the built `se mcp` in an isolated
//! environment and speaks JSON-RPC lines over the standard input and output
//! of the server, in either of the two lifecycles the server answers:
//!
//! - `Classic`: `initialize` with protocol 2025-06-18 and the notification
//!   `notifications/initialized`, as Codex starts a server;
//! - `Stateless`: `server/discover` with protocol 2026-07-28, after which
//!   every request carries the protocol version, the client data and the
//!   client capabilities in `_meta`, as the v2 runtime of Claude Code does.
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::cli::vault::task_env::TaskEnv;

/// How long the server may take for one reply.
const REPLY_LIMIT: Duration = Duration::from_secs(240);
/// How long the server may take to end after its input was closed.
const END_LIMIT: Duration = Duration::from_secs(30);

/// The two ways a client opens the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lifecycle {
    Classic,
    Stateless,
}

impl Lifecycle {
    /// The protocol version the client asks for.
    pub(super) fn version(self) -> &'static str {
        match self {
            Self::Classic => "2025-06-18",
            Self::Stateless => "2026-07-28",
        }
    }
}

/// The answer of a `tools/call`.
pub(super) struct ToolAnswer {
    /// `isError` of the result.
    pub(super) is_error: bool,
    /// `structuredContent` of the result.
    pub(super) data: Value,
}

/// The answer of the tool `se`: the command ran and ended.
pub(super) struct SeAnswer {
    pub(super) code: i64,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

impl fmt::Display for SeAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Exitcode {}\n--- stdout\n{}\n--- stderr\n{}",
            self.code, self.stdout, self.stderr
        )
    }
}

impl SeAnswer {
    /// The standard output as JSON.
    pub(super) fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|error| panic!("keine JSON-Ausgabe ({error}): {self}"))
    }
}

/// A running `se mcp` and the connection to it.
pub(super) struct McpClient {
    child: Child,
    input: Option<ChildStdin>,
    lines: Receiver<String>,
    log: Arc<Mutex<Vec<u8>>>,
    next_id: u64,
    lifecycle: Lifecycle,
    handshake: Value,
}

impl McpClient {
    /// Starts `se mcp` in `env` and opens the connection in `lifecycle`.
    pub(super) fn start(env: &TaskEnv, lifecycle: Lifecycle) -> Self {
        let mut command = env.command(&["mcp"]);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("se mcp startet");
        let input = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout-Pipe");
        let mut stderr = child.stderr.take().expect("stderr-Pipe");
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let log = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&log);
        thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match stderr.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if let Ok(mut bytes) = shared.lock() {
                            bytes.extend_from_slice(&chunk[..count]);
                        }
                    }
                }
            }
        });
        let mut client = Self {
            child,
            input,
            lines,
            log,
            next_id: 1,
            lifecycle,
            handshake: Value::Null,
        };
        client.handshake = client.open();
        client
    }

    /// What the server answered when the connection opened: the result of
    /// `initialize` (classic) or of `server/discover` (stateless).
    pub(super) fn handshake(&self) -> &Value {
        &self.handshake
    }

    fn open(&mut self) -> Value {
        match self.lifecycle {
            Lifecycle::Classic => {
                let version = self.lifecycle.version();
                let result = self.request(
                    "initialize",
                    json!({
                        "protocolVersion": version,
                        "capabilities": {},
                        "clientInfo": { "name": "vault-task", "version": "0" },
                    }),
                );
                self.send(&json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/initialized",
                }));
                result
            }
            Lifecycle::Stateless => self.request("server/discover", json!({})),
        }
    }

    /// The text of what the server wrote to standard error so far.
    fn log_text(&self) -> String {
        let bytes = self.log.lock().map(|bytes| bytes.clone());
        format!(
            "--- stderr von se mcp\n{}",
            String::from_utf8_lossy(&bytes.unwrap_or_default())
        )
    }

    fn send(&mut self, message: &Value) {
        let mut line = message.to_string();
        line.push('\n');
        let written = match self.input.as_mut() {
            Some(input) => input
                .write_all(line.as_bytes())
                .and_then(|()| input.flush()),
            None => panic!("die Eingabe des Servers ist geschlossen"),
        };
        if let Err(error) = written {
            panic!("senden: {error}\n{}", self.log_text());
        }
    }

    /// Sends the request `method` and returns its `result`; a notification
    /// or a reply to another request in between is skipped, an error reply
    /// fails the test. In the stateless lifecycle the request carries its
    /// client data in `_meta`.
    pub(super) fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let mut params = params;
        if self.lifecycle == Lifecycle::Stateless {
            if params.is_null() {
                params = json!({});
            }
            params["_meta"] = json!({
                "io.modelcontextprotocol/protocolVersion": Lifecycle::Stateless.version(),
                "io.modelcontextprotocol/clientInfo": { "name": "vault-task", "version": "0" },
                "io.modelcontextprotocol/clientCapabilities": {},
            });
        }
        let mut message = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if !params.is_null() {
            message["params"] = params;
        }
        self.send(&message);
        let deadline = Instant::now() + REPLY_LIMIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = match self.lines.recv_timeout(left) {
                Ok(line) => line,
                Err(error) => panic!("{method}: keine Antwort ({error})\n{}", self.log_text()),
            };
            let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if reply["id"] != id {
                continue;
            }
            if let Some(error) = reply.get("error") {
                panic!("{method}: Fehlerantwort {error}\n{}", self.log_text());
            }
            return reply["result"].clone();
        }
    }

    /// `tools/call` of the tool `name`.
    pub(super) fn call(&mut self, name: &str, arguments: Value) -> ToolAnswer {
        let result = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        ToolAnswer {
            is_error: result["isError"] == true,
            data: result["structuredContent"].clone(),
        }
    }

    /// The tool `se` with the arguments `args` and the standard input
    /// `stdin`: the command ended, whatever its exit code.
    pub(super) fn se(&mut self, args: &[&str], stdin: Option<&str>) -> SeAnswer {
        let mut arguments = json!({ "args": args, "wait_seconds": 120 });
        if let Some(text) = stdin {
            arguments["stdin"] = json!(text);
        }
        let answer = self.call("se", arguments);
        assert!(!answer.is_error, "se {args:?}: Fehler {}", answer.data);
        let data = answer.data;
        assert_eq!(data["status"], "completed", "se {args:?}: {data}");
        SeAnswer {
            code: data["exit_code"].as_i64().unwrap_or(-1),
            stdout: data["stdout"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            stderr: data["stderr"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        }
    }

    /// Closes the input of the server, as a client does when it ends, and
    /// waits for the server to end with exit code 0.
    pub(super) fn finish(mut self) {
        drop(self.input.take());
        let deadline = Instant::now() + END_LIMIT;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    assert!(
                        status.success(),
                        "se mcp endete mit {status}\n{}",
                        self.log_text()
                    );
                    return;
                }
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(50));
                }
                Ok(None) => panic!(
                    "se mcp endet nicht, nachdem seine Eingabe geschlossen wurde\n{}",
                    self.log_text()
                ),
                Err(error) => panic!("se mcp: Warten gescheitert: {error}"),
            }
        }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
