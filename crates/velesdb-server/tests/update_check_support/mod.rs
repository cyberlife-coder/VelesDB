//! Shared by the server's and the CLI's update-check wiring tests (the CLI
//! includes this file by path): a loopback listener standing in for
//! `velesdb.com`, a child-process guard, and an environment that cannot
//! touch the developer's home directory. Nothing here leaves the machine.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Kills and reaps the child when dropped, so a failing assertion cannot
/// leave a server running.
pub struct ChildGuard(pub Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A loopback HTTP endpoint that records the first request it receives.
pub struct Endpoint {
    listener: TcpListener,
}

impl Endpoint {
    pub fn bind() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        Self { listener }
    }

    pub fn url(&self) -> String {
        let port = self.listener.local_addr().expect("local addr").port();
        format!("http://127.0.0.1:{port}/api/check")
    }

    /// The first request received within `within` (headers and body), or
    /// `None`. The reply is a valid "up to date" answer.
    pub fn receive(&self, within: Duration) -> Option<String> {
        let deadline = Instant::now() + within;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => return Some(serve(stream)),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("accept failed: {e}"),
            }
        }
    }
}

fn serve(mut stream: TcpStream) -> String {
    stream.set_nonblocking(false).expect("blocking stream");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !is_complete(&request) {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => request.extend_from_slice(&chunk[..n]),
        }
    }
    let body = r#"{"latest_version":"0.0.0","update_available":false}"#;
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    String::from_utf8_lossy(&request).into_owned()
}

/// True once the headers and as many body bytes as `content-length` promises
/// have arrived.
fn is_complete(request: &[u8]) -> bool {
    let text = String::from_utf8_lossy(request);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let wanted = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    body.len() >= wanted
}

/// Writes a `velesdb.toml` whose `[update_check]` points at `endpoint`, with
/// `enabled` set when given.
pub fn write_config(dir: &Path, enabled: Option<bool>, endpoint: &str) -> PathBuf {
    let enabled_line = enabled.map_or_else(String::new, |value| format!("enabled = {value}\n"));
    let path = dir.join("velesdb.toml");
    std::fs::write(
        &path,
        format!("[update_check]\n{enabled_line}endpoint = \"{endpoint}\"\n"),
    )
    .expect("write config");
    path
}

/// Points every per-user location at `home` and drops the variables that
/// would opt out or redirect the config, so the child cannot read or write
/// the developer's real configuration.
pub fn isolate(command: &mut Command, home: &Path) {
    command
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("APPDATA", home)
        .env_remove("VELESDB_NO_UPDATE_CHECK")
        .env_remove("VELESDB_UPDATE_CHECK")
        .env_remove("VELESDB_CONFIG");
}
