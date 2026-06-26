//! Local generative client for a running Ollama (`/api/generate`).
//!
//! Used for both fact extraction and answer judging with a local model
//! (default `qwen3.6:35b-mlx`). Every call is content-addressed and cached on
//! disk: an interrupted multi-hour run resumes for free, and re-runs spend no
//! GPU. `think` is disabled and `temperature` pinned to 0 for stable output.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const DEFAULT_MODEL: &str = "qwen3.6:35b-mlx";
const OLLAMA_URL: &str = "http://localhost:11434/api/generate";
const ATTEMPTS: u32 = 3;

/// Per-process counter making temp-file names unique, so two runs sharing a
/// cache dir never write the same `.tmp` and race the rename.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A cached, deterministic text generator backed by local Ollama.
pub struct Generator {
    model: String,
    cache_dir: PathBuf,
    agent: ureq::Agent,
}

impl Generator {
    /// Build a generator. `model` falls back to [`DEFAULT_MODEL`] when empty;
    /// `cache_dir` is created if missing.
    pub fn new(model: &str, cache_dir: PathBuf) -> Result<Self, Box<dyn Error>> {
        fs::create_dir_all(&cache_dir)?;
        let model = if model.is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            model.to_string()
        };
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(300))
            .build();
        Ok(Self {
            model,
            cache_dir,
            agent,
        })
    }

    /// The active model name, for report headers.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Generate text for `prompt`, served from cache when seen before. An empty
    /// cached entry is treated as a miss, so a transient blank reply is never
    /// trusted as a permanent answer.
    pub fn generate(&self, prompt: &str) -> Result<String, Box<dyn Error>> {
        let key = self.key(prompt);
        let path = self.cache_dir.join(format!("{key}.txt"));
        if let Ok(cached) = fs::read_to_string(&path) {
            if !cached.trim().is_empty() {
                return Ok(cached);
            }
        }
        let answer = self.call(prompt)?;
        if !answer.trim().is_empty() {
            self.store(&key, &path, &answer)?;
        }
        Ok(answer)
    }

    /// Atomically persist `answer`: write a process-unique temp file, then
    /// rename it over `path`, so a killed run never leaves a half-written entry.
    fn store(&self, key: &str, path: &Path, answer: &str) -> Result<(), Box<dyn Error>> {
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = self
            .cache_dir
            .join(format!("{key}.{}.{seq}.tmp", std::process::id()));
        fs::write(&tmp, answer)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// POST to Ollama with bounded retries; returns the trimmed `response`.
    /// The body is serialised with `serde_json` and sent as a raw string so the
    /// crate's `ureq` can keep `default-features = false` (no bundled TLS/json),
    /// leaving the shipped server binary tiny.
    fn call(&self, prompt: &str) -> Result<String, Box<dyn Error>> {
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "think": false,
            "options": { "temperature": 0 },
        })
        .to_string();
        let mut last: Box<dyn Error> = "no attempt made".into();
        for attempt in 1..=ATTEMPTS {
            let request = self
                .agent
                .post(OLLAMA_URL)
                .set("Content-Type", "application/json");
            match request.send_string(&body) {
                Ok(resp) => return parse_response(resp),
                Err(e) => last = ureq_error(attempt, &e).into(),
            }
        }
        Err(last)
    }

    /// Content-addressed cache key over model + prompt (stable across runs).
    fn key(&self, prompt: &str) -> String {
        let mut hash = fnv1a(self.model.as_bytes());
        hash = fnv1a_continue(hash, b"\0");
        hash = fnv1a_continue(hash, prompt.as_bytes());
        format!("{hash:016x}")
    }
}

/// Pull the `response` string out of Ollama's JSON envelope.
fn parse_response(resp: ureq::Response) -> Result<String, Box<dyn Error>> {
    let raw = resp.into_string()?;
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let text = value
        .get("response")
        .and_then(serde_json::Value::as_str)
        .ok_or("Ollama reply had no `response` field")?;
    Ok(text.trim().to_string())
}

/// Flatten a `ureq` error into a labelled message for the retry log.
fn ureq_error(attempt: u32, error: &ureq::Error) -> String {
    format!("Ollama request failed (attempt {attempt}/{ATTEMPTS}): {error}")
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over a single byte slice — a stable, dependency-free cache hash.
fn fnv1a(bytes: &[u8]) -> u64 {
    fnv1a_continue(FNV_OFFSET, bytes)
}

/// Continue an FNV-1a hash with more bytes.
fn fnv1a_continue(mut hash: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
