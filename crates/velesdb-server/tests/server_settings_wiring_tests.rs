//! The `[server]` and `[logging]` settings `docs/guides/CONFIGURATION.md`
//! documents, end to end: each test starts the real binary with a
//! `--config` file and watches the behaviour from outside — the status of an
//! oversized request, the CORS headers a browser would see, what lands in the
//! log file. A unit test of the config parser cannot catch a `main.rs` that
//! parses a value and then never hands it to the router, the runtime or the
//! subscriber; for years `workers`, `max_body_size`, `cors_enabled`,
//! `cors_origins` and the whole `[logging]` table were exactly that, minus the
//! parsing.

#![cfg(feature = "persistence")]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const WAIT: Duration = Duration::from_secs(60);

/// Kills and reaps the server when dropped, so a failing assertion cannot
/// leave one running.
struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

fn write_config(home: &Path, toml: &str) -> PathBuf {
    let path = home.join("velesdb.toml");
    std::fs::write(&path, toml).expect("write config");
    path
}

/// Starts the server in `home` with `config` and the extra environment
/// `env`, and waits until it accepts connections. Every variable that could
/// change the outcome from the developer's shell is cleared first, and the
/// update check is off so nothing leaves the machine.
fn start(home: &TempDir, config: &str, env: &[(&str, &str)]) -> Server {
    let config_path = write_config(home.path(), config);
    start_with(home, Some(config_path.as_path()), env)
}

/// [`start`] without writing a file; with `config_path: None` the server
/// gets no `--config` and its working directory holds no `velesdb.toml`.
fn start_with(home: &TempDir, config_path: Option<&Path>, env: &[(&str, &str)]) -> Server {
    let port = free_port();
    let mut command = Command::new(env!("CARGO_BIN_EXE_velesdb-server"));
    command
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("VELESDB_NO_UPDATE_CHECK", "1")
        .env_remove("RUST_LOG")
        .env_remove("VELESDB_LOGGING_LEVEL")
        .env_remove("VELESDB_CONFIG")
        .env_remove("VELESDB_API_KEYS")
        .envs(env.iter().copied())
        .args(
            config_path
                .map(|path| vec!["--config".to_string(), path.to_string_lossy().into_owned()])
                .unwrap_or_default(),
        )
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .args(["--data-dir", &home.path().join("data").to_string_lossy()])
        .current_dir(home.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let server = Server {
        child: command.spawn().expect("spawn velesdb-server"),
        port,
    };
    let deadline = Instant::now() + WAIT;
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "the server never came up");
        std::thread::sleep(Duration::from_millis(50));
    }
    server
}

/// Sends one HTTP/1.1 request and returns the raw response, headers
/// lower-cased so header assertions do not depend on the server's casing.
fn send(server: &Server, method: &str, path: &str, headers: &[&str], body: &[u8]) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", server.port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\
         content-type: application/json\r\ncontent-length: {}\r\n",
        body.len()
    );
    for header in headers {
        head.push_str(header);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    // A server that rejects a body before reading all of it may close the
    // socket mid-write; what it answered is still readable below.
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let mut response = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => response.extend_from_slice(&chunk[..n]),
        }
    }
    String::from_utf8_lossy(&response).to_ascii_lowercase()
}

fn status_of(response: &str) -> &str {
    response.split(' ').nth(1).unwrap_or_default()
}

fn allow_origin(response: &str) -> Option<&str> {
    response
        .lines()
        .find_map(|line| line.strip_prefix("access-control-allow-origin:"))
        .map(str::trim)
}

/// Polls the log file until a line contains `needle`, then returns the file.
fn wait_for_log(path: &Path, needle: &str) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        let contents = std::fs::read_to_string(path).unwrap_or_default();
        if contents.contains(needle) {
            return contents;
        }
        assert!(
            Instant::now() < deadline,
            "{needle:?} never reached {}:\n{contents}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

// ---------------------------------------------------------------------------
// [server] max_body_size
// ---------------------------------------------------------------------------

#[test]
fn test_a_body_above_max_body_size_is_refused_with_413_on_every_route() {
    let home = TempDir::new().expect("tempdir");
    let server = start(&home, "[server]\nmax_body_size = 4096\n", &[]);
    let oversized = vec![b' '; 5000];

    // Positive control first: a small body reaches the handler, which answers
    // for the missing collection — so a 413 below is the limit, not a server
    // that refuses everything.
    let small = send(
        &server,
        "POST",
        "/collections/none/points",
        &[],
        b"{\"points\":[]}",
    );
    assert_eq!(status_of(&small), "404", "{small}");

    let upload = send(&server, "POST", "/collections/none/points", &[], &oversized);
    assert_eq!(status_of(&upload), "413", "{upload}");
    // Below the 2 MiB that every non-upload route keeps, the setting caps
    // those too: it is the largest body the server accepts anywhere.
    let query = send(&server, "POST", "/query", &[], &oversized);
    assert_eq!(status_of(&query), "413", "{query}");
}

// ---------------------------------------------------------------------------
// [server] cors_enabled / cors_origins
// ---------------------------------------------------------------------------

#[test]
fn test_cors_origins_allow_a_listed_origin_and_nothing_that_merely_resembles_it() {
    let home = TempDir::new().expect("tempdir");
    let server = start(
        &home,
        "[server]\ncors_origins = [\"https://app.example.com\"]\n",
        &[],
    );

    let listed = send(
        &server,
        "GET",
        "/health",
        &["origin: https://app.example.com"],
        b"",
    );
    assert_eq!(status_of(&listed), "200", "{listed}");
    assert_eq!(
        allow_origin(&listed),
        Some("https://app.example.com"),
        "{listed}"
    );

    for refused in [
        "https://evil.example.com",
        "https://app.example.com.evil.net",
        "http://app.example.com",
    ] {
        let header = format!("origin: {refused}");
        let response = send(&server, "GET", "/health", &[&header], b"");
        assert_eq!(
            allow_origin(&response),
            None,
            "{refused} was allowed:\n{response}"
        );
    }
}

#[test]
fn test_cors_enabled_false_sends_no_cors_headers_at_all() {
    let home = TempDir::new().expect("tempdir");
    let server = start(&home, "[server]\ncors_enabled = false\n", &[]);

    let response = send(
        &server,
        "GET",
        "/health",
        &["origin: https://app.example.com"],
        b"",
    );
    assert_eq!(status_of(&response), "200", "{response}");
    assert_eq!(allow_origin(&response), None, "{response}");
}

/// The default is same-origin only: with no origin configured the server
/// sends no CORS header, so no other web page can read its responses.
#[test]
fn test_without_cors_settings_no_cors_header_is_sent() {
    let home = TempDir::new().expect("tempdir");
    let server = start(&home, "[server]\n", &[]);

    let response = send(
        &server,
        "GET",
        "/health",
        &["origin: https://app.example.com"],
        b"",
    );
    assert_eq!(status_of(&response), "200", "{response}");
    assert_eq!(allow_origin(&response), None, "{response}");
}

/// Allow-all is still available, but only when written down.
#[test]
fn test_an_explicit_wildcard_allows_every_origin() {
    let home = TempDir::new().expect("tempdir");
    let server = start(&home, "[server]\ncors_origins = [\"*\"]\n", &[]);

    let response = send(
        &server,
        "GET",
        "/health",
        &["origin: https://app.example.com"],
        b"",
    );
    assert_eq!(allow_origin(&response), Some("*"), "{response}");
}

// ---------------------------------------------------------------------------
// [logging] + [server] workers + the unread-key warning
// ---------------------------------------------------------------------------

#[test]
fn test_json_logging_appends_to_the_file_and_reports_workers_and_unread_keys() {
    let home = TempDir::new().expect("tempdir");
    let log = home.path().join("velesdb.log");
    std::fs::write(&log, "line from a previous run\n").expect("seed log file");
    let config = format!(
        "[server]\nworkers = 2\nnot_a_server_key = 1\n\n\
         [logging]\nformat = \"json\"\nfile = \"{}\"\n",
        log.display()
    );
    let _server = start(&home, &config, &[]);

    let contents = wait_for_log(&log, "listening");
    let mut lines = contents.lines();
    assert_eq!(
        lines.next(),
        Some("line from a previous run"),
        "appended, not truncated"
    );
    for line in lines {
        let event: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"));
        assert!(
            event["level"].is_string() && event["fields"].is_object(),
            "{line}"
        );
    }
    assert!(contents.contains("server.not_a_server_key"), "{contents}");
    assert!(contents.contains("2 worker thread(s)"), "{contents}");
}

#[test]
fn test_velesdb_logging_level_overrides_the_file_level() {
    let home = TempDir::new().expect("tempdir");
    let log = home.path().join("velesdb.log");
    let config = format!(
        "[logging]\nlevel = \"error\"\nfile = \"{}\"\n",
        log.display()
    );
    let _server = start(&home, &config, &[("VELESDB_LOGGING_LEVEL", "info")]);

    // `listening` is an INFO event: the file alone ("error") would hide it.
    let contents = wait_for_log(&log, "listening");
    assert!(
        !contents.contains("\u{1b}["),
        "no ANSI colours in a file:\n{contents}"
    );
}

// ---------------------------------------------------------------------------
// VELESDB_SERVER_* / VELESDB_LOGGING_* belong to the server, not the engine
// ---------------------------------------------------------------------------

/// `VELESDB_SERVER_PORT` is not a server variable (the port is
/// `VELESDB_PORT`), and the engine loader must not read it either: with a
/// config file present it used to reach core's own `server.port` and stop
/// the server with `server.port must be >= 1024`.
#[test]
fn test_a_shell_owned_variable_does_not_reach_the_engine_loader() {
    let home = TempDir::new().expect("tempdir");
    let server = start(
        &home,
        "[search]\ndefault_mode = \"fast\"\n",
        &[
            ("VELESDB_SERVER_PORT", "80"),
            ("VELESDB_SERVER_WORKERS", "999999"),
        ],
    );

    let response = send(&server, "GET", "/health", &[], b"");
    assert_eq!(status_of(&response), "200", "{response}");
}

/// CONFIGURATION.md: `VELESDB_*` engine variables apply on top of the file —
/// and with no file at all. Without `--config` and without `./velesdb.toml`
/// the server used to start from `VelesConfig::default()` and skip the
/// environment, so `VELESDB_LIMITS_MAX_COLLECTIONS=1` did nothing.
#[test]
fn test_engine_variables_apply_without_any_config_file() {
    let home = TempDir::new().expect("tempdir");
    let server = start_with(&home, None, &[("VELESDB_LIMITS_MAX_COLLECTIONS", "1")]);

    let create = |name: &str| {
        let body = format!("{{\"name\":\"{name}\",\"dimension\":2,\"metric\":\"cosine\"}}");
        send(&server, "POST", "/collections", &[], body.as_bytes())
    };
    let first = create("first");
    assert_eq!(status_of(&first), "201", "{first}");
    let second = create("second");
    assert_ne!(
        status_of(&second),
        "201",
        "the limit from the environment: {second}"
    );
    assert!(second.contains("max_collections"), "{second}");
}
