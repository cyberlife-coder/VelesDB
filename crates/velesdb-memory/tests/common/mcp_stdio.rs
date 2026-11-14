//! A blocking JSON-RPC client driving the real `velesdb-memory` binary over
//! stdio — the transport every MCP harness (Claude Code, Codex, Cursor…)
//! uses. Process-level suites share it so each one exercises the shipped
//! binary instead of re-implementing the framing.
//!
//! Anti-orphan discipline: a client that is dropped without
//! [`McpStdioProcess::shutdown`] (a panicking test) kills and reaps its child.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

/// One live `velesdb-memory` server process and its stdio pipes.
pub struct McpStdioProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpStdioProcess {
    /// The server command for `store`, isolated from the developer's machine:
    /// `HOME` points at `home`, the offline `hash` embedder is pinned, and no
    /// ambient config file is read.
    pub fn command(store: &Path, home: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_velesdb-memory"));
        command
            .env("HOME", home)
            .env("VELESDB_MEMORY_PATH", store)
            .env("VELESDB_MEMORY_EMBEDDER", "hash")
            .env("VELESDB_MEMORY_QUIET", "1")
            .env_remove("VELESDB_MEMORY_CONFIG");
        command
    }

    /// Spawns `command` and completes the MCP `initialize` handshake.
    pub fn spawn(mut command: Command, client_name: &str) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn velesdb-memory");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let mut process = Self {
            child,
            stdin: Some(stdin),
            stdout,
            next_id: 1,
        };
        process.initialize(client_name);
        process
    }

    /// OS process id of the server.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Calls MCP tool `name` and returns its `structuredContent`, failing the
    /// test if the tool reports an error.
    pub fn call(&mut self, name: &str, arguments: Value) -> Value {
        let mut params = json!({"name":name});
        params["arguments"] = arguments;
        let response = self.request("tools/call", params);
        assert_ne!(
            response["result"]["isError"], true,
            "tool {name}: {response}"
        );
        response["result"]["structuredContent"].clone()
    }

    /// Sends one JSON-RPC request and returns its response.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let mut frame = json!({"jsonrpc":"2.0","id":id,"method":method});
        frame["params"] = params;
        self.send(&frame);
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("read response");
        let response: Value = serde_json::from_str(&line).expect("JSON response");
        assert_eq!(response["id"], id, "unexpected response: {response}");
        assert!(
            response.get("error").is_none(),
            "request failed: {response}"
        );
        response
    }

    /// Closes stdin — what a harness does when its session ends — and asserts
    /// the server exits cleanly, releasing the store lock.
    pub fn shutdown(&mut self) {
        drop(self.stdin.take());
        let status = self.child.wait().expect("server exit");
        assert!(status.success(), "server status: {status}");
    }

    fn initialize(&mut self, client_name: &str) {
        self.request(
            "initialize",
            json!({
                "protocolVersion":"2024-11-05",
                "capabilities":{},
                "clientInfo":{"name":client_name,"version":"1"}
            }),
        );
        self.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }

    fn send(&mut self, frame: &Value) {
        let stdin = self.stdin.as_mut().expect("live stdin");
        serde_json::to_writer(&mut *stdin, frame).expect("write frame");
        stdin.write_all(b"\n").expect("newline");
        stdin.flush().expect("flush");
    }
}

impl Drop for McpStdioProcess {
    fn drop(&mut self) {
        if self.stdin.is_some() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
