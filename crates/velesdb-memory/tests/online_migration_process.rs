#![cfg(all(feature = "mcp", feature = "persistence"))]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use velesdb_memory::{HashEmbedder, MemoryService, Metadata, DEFAULT_DIMENSION};

const TIMEOUT: Duration = Duration::from_secs(90);

#[test]
fn daemon_migration_preserves_writes_from_every_memory_path() {
    let root = tempfile::tempdir().expect("root");
    let store = root.path().join("store");
    let seed = seed_store(&store);
    let mut client = ProcessClient::spawn(&store, root.path(), true);

    client.start_migration();
    write_during_base_copy(&mut client, &seed);
    client.wait_for_phase("catching_up");
    let feedback = write_during_catch_up(&mut client, &seed);
    let _ = client.wait_for_commit();

    assert_post_cutover(&mut client, &seed, feedback);
    client.shutdown();
}

#[test]
#[ignore = "performance evidence; run deliberately with --release --nocapture"]
fn reports_steady_state_capture_and_replay_overhead() {
    let root = tempfile::tempdir().expect("root");
    let store = root.path().join("store");
    let _seed = seed_store(&store);
    let mut client = ProcessClient::spawn(&store, root.path(), false);
    let baseline = timed_remembers(&mut client, "baseline", 16);
    let migration_started = Instant::now();
    client.start_migration();
    let captured = timed_remembers(&mut client, "captured", 16);
    let committed = client.wait_for_commit();
    let elapsed = migration_started.elapsed();
    let replayed = committed["job"]["progress"]["output_watermark"]
        .as_u64()
        .expect("output watermark");
    println!(
        "baseline_us_per_write={:.1} capture_us_per_write={:.1} capture_ratio={:.2} replayed_records={} migration_ms={}",
        micros_per_write(baseline, 16),
        micros_per_write(captured, 16),
        captured.as_secs_f64() / baseline.as_secs_f64(),
        replayed,
        elapsed.as_millis()
    );
    client.shutdown();
}

fn timed_remembers(client: &mut ProcessClient, prefix: &str, count: u64) -> Duration {
    let started = Instant::now();
    for index in 0..count {
        client.call("remember", json!({"fact":format!("{prefix} fact {index}")}));
    }
    started.elapsed()
}

fn micros_per_write(elapsed: Duration, count: u64) -> f64 {
    let count = u32::try_from(count).expect("benchmark count fits");
    elapsed.as_secs_f64() * 1_000_000.0 / f64::from(count)
}

struct Seed {
    update_id: u64,
    delete_id: u64,
    edge_from: u64,
    edge_to: u64,
}

fn seed_store(path: &std::path::Path) -> Seed {
    let service = MemoryService::open(path, HashEmbedder::new(DEFAULT_DIMENSION)).expect("store");
    for index in 0..64 {
        service
            .remember(&format!("migration seed {index:04}"), &[], None)
            .expect("seed fact");
    }
    let update_id = service
        .remember(
            "mutable process fact",
            &[],
            Some(&metadata("version", "before")),
        )
        .expect("mutable fact");
    let delete_id = service
        .remember("delete during migration", &[], None)
        .expect("delete fact");
    let edge_from = service
        .remember("edge source", &[], None)
        .expect("edge source");
    let edge_to = service
        .remember("edge target", &[], None)
        .expect("edge target");
    drop(service);
    Seed {
        update_id,
        delete_id,
        edge_from,
        edge_to,
    }
}

fn metadata(key: &str, value: &str) -> Metadata {
    [(key.to_owned(), Value::String(value.to_owned()))]
        .into_iter()
        .collect()
}

fn write_during_base_copy(client: &mut ProcessClient, seed: &Seed) {
    client.call(
        "remember",
        json!({"fact":"created during base copy","metadata":{"stage":"base"}}),
    );
    client.call("forget", json!({"id":seed.delete_id.to_string()}));
    client.call(
        "relate",
        json!({
            "from":seed.edge_from.to_string(),
            "to":seed.edge_to.to_string(),
            "relation":"temporary-process-edge"
        }),
    );
}

fn write_during_catch_up(client: &mut ProcessClient, seed: &Seed) -> f64 {
    client.call(
        "remember",
        json!({
            "fact":"mutable process fact",
            "metadata":{"version":"after"},
            "ttl_seconds":3600
        }),
    );
    client.call(
        "unrelate",
        json!({
            "from":seed.edge_from.to_string(),
            "to":seed.edge_to.to_string(),
            "relation":"temporary-process-edge"
        }),
    );
    let feedback = client.call(
        "feedback",
        json!({"id":seed.update_id.to_string(),"success":true}),
    );
    client.call(
        "remember",
        json!({"fact":"fact: autograph survived | process-autograph"}),
    );
    let receipt = client.call(
        "remember_extracted",
        json!({
            "text":"fact: extracted survived | process-target\nedge: process-target | validates | migration\nattr: process-target | verified | true",
            "extractor":"outline",
            "idempotency_key":"online-migration-process"
        }),
    );
    client.wait_for_extraction(text(&receipt, "request_id"));
    number(&feedback, "confidence")
}

fn assert_post_cutover(client: &mut ProcessClient, seed: &Seed, first_confidence: f64) {
    let memories = client.list_all();
    assert!(has_content(&memories, "created during base copy"));
    assert!(has_content(&memories, "extracted survived"));
    assert!(!has_id(&memories, seed.delete_id));
    let updated = memory_by_id(&memories, seed.update_id);
    assert_eq!(updated["metadata"]["version"], "after");
    assert!(updated["metadata"]["_veles_expires_at"].as_u64().is_some());

    let removed = client.call(
        "unrelate",
        json!({
            "from":seed.edge_from.to_string(),
            "to":seed.edge_to.to_string(),
            "relation":"temporary-process-edge"
        }),
    );
    assert_eq!(removed["found"], false);
    let reinforced = client.call(
        "feedback",
        json!({"id":seed.update_id.to_string(),"success":true}),
    );
    assert!(number(&reinforced, "confidence") > first_confidence);
    let entity = client.call("entity", json!({"name":"process-target"}));
    assert_eq!(entity["found"], true);
}

fn has_content(memories: &[Value], expected: &str) -> bool {
    memories.iter().any(|memory| memory["content"] == expected)
}

fn has_id(memories: &[Value], expected: u64) -> bool {
    let expected = expected.to_string();
    memories
        .iter()
        .any(|memory| memory["id_str"].as_str() == Some(expected.as_str()))
}

fn memory_by_id(memories: &[Value], expected: u64) -> &Value {
    let expected = expected.to_string();
    memories
        .iter()
        .find(|memory| memory["id_str"].as_str() == Some(expected.as_str()))
        .expect("memory id")
}

struct ProcessClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl ProcessClient {
    fn spawn(store: &std::path::Path, home: &std::path::Path, extractor: bool) -> Self {
        let mut command = daemon_command(store, home);
        if extractor {
            command.env("VELESDB_MEMORY_EXTRACTOR", "outline");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn daemon");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let mut client = Self {
            child,
            stdin: Some(stdin),
            stdout,
            next_id: 1,
        };
        client.initialize();
        client
    }

    fn initialize(&mut self) {
        self.request(
            "initialize",
            json!({
                "protocolVersion":"2024-11-05",
                "capabilities":{},
                "clientInfo":{"name":"online-migration-process","version":"1"}
            }),
        );
        self.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }

    fn start_migration(&mut self) {
        self.call(
            "migration_start",
            json!({
                "target_backend":"hash",
                "pause_budget_ms":5000,
                "fact_batch":1,
                "replay_batch":1,
                "edge_cap":64,
                "observation_window":8,
                "verification_reserve_ms":10
            }),
        );
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let mut params = json!({"name":name});
        params["arguments"] = arguments;
        let response = self.request("tools/call", params);
        assert_ne!(
            response["result"]["isError"], true,
            "tool {name}: {response}"
        );
        response["result"]["structuredContent"].clone()
    }

    fn wait_for_phase(&mut self, expected: &str) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let status = self.call("migration_status", json!({}));
            let phase = status["job"]["phase"].as_str().unwrap_or_default();
            if phase == expected {
                return;
            }
            assert!(
                status["job"]["last_error"].is_null(),
                "migration failed: {status}"
            );
            assert!(
                Instant::now() < deadline,
                "status {status}, expected {expected}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for_extraction(&mut self, request_id: &str) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let status = self.call("extraction_status", json!({"request_id":request_id}));
            match status["state"].as_str() {
                Some("committed") => return,
                Some("failed") => panic!("extraction failed: {status}"),
                _ => {}
            }
            assert!(Instant::now() < deadline, "extraction did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for_commit(&mut self) -> Value {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let status = self.call("migration_status", json!({}));
            let phase = status["job"]["phase"].as_str().unwrap_or_default();
            if phase == "committed" {
                return status;
            }
            if phase == "non_converging" && status["job"]["running"] == false {
                self.call("migration_recover", json!({}));
            }
            assert!(
                status["job"]["last_error"].is_null(),
                "migration failed: {status}"
            );
            assert!(
                Instant::now() < deadline,
                "migration did not commit: {status}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn list_all(&mut self) -> Vec<Value> {
        let mut cursor: Option<String> = None;
        let mut memories = Vec::new();
        loop {
            let page = self.call(
                "list_memories",
                json!({"cursor":cursor,"limit":100,"include_internal":true}),
            );
            memories.extend(
                page["memories"]
                    .as_array()
                    .expect("memories")
                    .iter()
                    .cloned(),
            );
            cursor = page["next_cursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                return memories;
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
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

    fn send(&mut self, frame: &Value) {
        let stdin = self.stdin.as_mut().expect("live stdin");
        serde_json::to_writer(&mut *stdin, frame).expect("write frame");
        stdin.write_all(b"\n").expect("newline");
        stdin.flush().expect("flush");
    }

    fn shutdown(&mut self) {
        drop(self.stdin.take());
        let status = self.child.wait().expect("daemon exit");
        assert!(status.success(), "daemon status: {status}");
    }
}

fn daemon_command(store: &std::path::Path, home: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_velesdb-memory"));
    command
        .env("HOME", home)
        .env("VELESDB_MEMORY_PATH", store)
        .env("VELESDB_MEMORY_EMBEDDER", "hash")
        .env("VELESDB_MEMORY_QUIET", "1")
        .env_remove("VELESDB_MEMORY_CONFIG");
    command
}

impl Drop for ProcessClient {
    fn drop(&mut self) {
        if self.stdin.is_some() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("missing {key}: {value}"))
}

fn number(value: &Value, key: &str) -> f64 {
    value[key]
        .as_f64()
        .unwrap_or_else(|| panic!("missing {key}: {value}"))
}
