//! `velesdb serve` (#2725): the velesdb-server bootstrap in-process.
//!
//! Two halves, each compiled only under the matching feature set: with
//! `serve`, the embedded server answers HTTP and exits on SIGTERM; without
//! it, the subcommand fails with a pointer to the feature.

use assert_cmd::Command;
#[cfg(not(feature = "serve"))]
use predicates::prelude::*;
#[cfg(feature = "serve")]
use std::net::TcpListener;
#[cfg(feature = "serve")]
use tempfile::TempDir;

fn velesdb() -> Command {
    let mut cmd = Command::cargo_bin("velesdb").unwrap();
    cmd.env_remove("VELESDB_CONFIG")
        .env("VELESDB_NO_UPDATE_CHECK", "1");
    cmd
}

#[test]
#[cfg(not(feature = "serve"))]
fn serve_without_the_feature_fails_with_a_pointer() {
    velesdb()
        .arg("serve")
        .assert()
        .failure()
        .stderr(predicate::str::contains("serve` cargo feature"));
}

#[cfg(feature = "serve")]
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[cfg(feature = "serve")]
fn get(port: u16, path: &str) -> Option<String> {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .write_all(format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
        .ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    Some(raw)
}

/// The embedded server answers the versioned API, serves the same database
/// files the CLI writes, and stops on SIGTERM (flushing on the way out).
#[test]
#[cfg(feature = "serve")]
fn serve_answers_rest_and_stops_on_sigterm() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("db");
    velesdb()
        .args(["collection", "create"])
        .arg(&db)
        .args(["docs", "--dimension", "4"])
        .assert()
        .success();

    let port = free_port();
    let bin = assert_cmd::cargo::cargo_bin("velesdb");
    let mut child = std::process::Command::new(bin)
        .arg("serve")
        .args(["-d", &db.to_string_lossy()])
        .args(["--host", "127.0.0.1", "-p", &port.to_string()])
        .env_remove("VELESDB_CONFIG")
        .env("VELESDB_NO_UPDATE_CHECK", "1")
        .spawn()
        .unwrap();

    let mut up = false;
    for _ in 0..60 {
        if get(port, "/v1/health").is_some() {
            up = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    assert!(up, "embedded server did not start on port {port}");

    assert!(get(port, "/v1/collections").is_some_and(|b| b.contains("docs")));
    assert!(get(port, "/v1/health").is_some_and(|b| b.contains(env!("CARGO_PKG_VERSION"))));

    libc_kill(child.id());
    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "server should exit 0 on SIGTERM: {status}"
    );
    assert!(status.success(), "exit status: {status}");
}

#[cfg(feature = "serve")]
fn libc_kill(pid: u32) {
    // `Child::kill` sends SIGKILL; the shutdown path is what this test
    // exercises, so use the platform `kill` for SIGTERM.
    std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .unwrap();
}
