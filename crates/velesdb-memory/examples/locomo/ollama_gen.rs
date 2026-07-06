//! Local generative client for a running Ollama (`/api/generate`).
//!
//! Used for both fact extraction and answer judging with a local model
//! (default `qwen3.6:35b-mlx`). Every call is content-addressed and cached on
//! disk: an interrupted multi-hour run resumes for free, and re-runs spend no
//! GPU. `think` is disabled and `temperature` pinned to 0 for stable output.

use std::error::Error;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const DEFAULT_MODEL: &str = "qwen3.6:35b-mlx";
const OLLAMA_URL: &str = "http://localhost:11434/api/generate";
const ATTEMPTS: u32 = 3;

/// The judging model, run through the authenticated `claude` CLI (no API key /
/// HTTPS client needed). A stronger, vendor-neutral judge than the local model,
/// aligning accuracy numbers with how published `LoCoMo` results are scored.
const JUDGE_MODEL: &str = "claude-opus-4-8";

/// Per-process counter making temp-file names unique, so two runs sharing a
/// cache dir never write the same `.tmp` and race the rename.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A cached, deterministic text generator backed by local Ollama.
pub struct Generator {
    model: String,
    cache_dir: PathBuf,
    agent: ureq::Agent,
}

/// Ollama's reported token usage for one generation call. Only available on a
/// live call (Ollama's `/api/generate` response), never on a cache hit or a
/// Claude-CLI call — callers must treat `None` as "not recorded", not "zero".
#[derive(Clone, Copy)]
pub struct TokenUsage {
    pub prompt: u64,
    pub completion: u64,
}

/// One live-call reply: the answer text plus whatever usage counters Ollama
/// reported alongside it.
struct GenReply {
    text: String,
    prompt_eval_count: Option<u64>,
    eval_count: Option<u64>,
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
        Ok(self.generate_traced(prompt)?.0)
    }

    /// Same as [`Self::generate`], but also returns Ollama's reported token
    /// usage when this call actually reached the model. A cache hit carries no
    /// usage counters — `None`, honestly, rather than a guessed value.
    pub fn generate_traced(
        &self,
        prompt: &str,
    ) -> Result<(String, Option<TokenUsage>), Box<dyn Error>> {
        self.generate_ctx(prompt, None)
    }

    /// Like [`Self::generate_traced`], but pins Ollama's context window to
    /// `num_ctx` tokens for this call — needed by the full-context baseline,
    /// whose whole-conversation prompt overflows Ollama's small default window
    /// (a silent truncation there would invalidate the ceiling it measures).
    pub fn generate_full_ctx(
        &self,
        prompt: &str,
        num_ctx: u64,
    ) -> Result<(String, Option<TokenUsage>), Box<dyn Error>> {
        self.generate_ctx(prompt, Some(num_ctx))
    }

    /// Cache-then-call core shared by the default and pinned-context entries.
    fn generate_ctx(
        &self,
        prompt: &str,
        num_ctx: Option<u64>,
    ) -> Result<(String, Option<TokenUsage>), Box<dyn Error>> {
        let key = self.key_ctx(prompt, num_ctx);
        let path = self.cache_dir.join(format!("{key}.txt"));
        if let Ok(cached) = fs::read_to_string(&path) {
            if !cached.trim().is_empty() {
                return Ok((cached, None));
            }
        }
        let reply = self.call(prompt, num_ctx)?;
        if !reply.text.trim().is_empty() {
            self.store(&key, &path, &reply.text)?;
        }
        let usage = match (reply.prompt_eval_count, reply.eval_count) {
            (Some(prompt), Some(completion)) => Some(TokenUsage { prompt, completion }),
            _ => None,
        };
        Ok((reply.text, usage))
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

    /// POST to Ollama with bounded retries; returns the trimmed `response` plus
    /// any usage counters. The body is serialised with `serde_json` and sent as
    /// a raw string so the crate's `ureq` can keep `default-features = false`
    /// (no bundled TLS/json), leaving the shipped server binary tiny.
    fn call(&self, prompt: &str, num_ctx: Option<u64>) -> Result<GenReply, Box<dyn Error>> {
        let mut options = serde_json::json!({ "temperature": 0 });
        if let Some(tokens) = num_ctx {
            options["num_ctx"] = serde_json::json!(tokens);
        }
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "think": false,
            "options": options,
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

    /// Judge `prompt` with Claude Opus 4.8 via the `claude` CLI, cached on disk
    /// keyed by the judge model so its verdicts never collide with the local
    /// model's (and a re-judge reuses cached answers, only the verdicts re-run).
    pub fn judge(&self, prompt: &str) -> Result<String, Box<dyn Error>> {
        let key = self.key_for(JUDGE_MODEL, prompt);
        let path = self.cache_dir.join(format!("{key}.txt"));
        if let Ok(cached) = fs::read_to_string(&path) {
            if !cached.trim().is_empty() {
                return Ok(cached);
            }
        }
        let answer = self.call_claude(prompt)?;
        if !answer.trim().is_empty() {
            self.store(&key, &path, &answer)?;
        }
        Ok(answer)
    }

    /// Invoke `claude -p --model <JUDGE_MODEL>`, feeding the prompt on stdin and
    /// returning trimmed stdout, with bounded retries for transient failures.
    fn call_claude(&self, prompt: &str) -> Result<String, Box<dyn Error>> {
        let mut last: Box<dyn Error> = "no attempt made".into();
        for attempt in 1..=ATTEMPTS {
            match self.claude_once(prompt) {
                Ok(text) => return Ok(text),
                Err(e) => {
                    last =
                        format!("claude judge failed (attempt {attempt}/{ATTEMPTS}): {e}").into();
                }
            }
        }
        Err(last)
    }

    /// One `claude -p` invocation.
    #[allow(clippy::unused_self)]
    fn claude_once(&self, prompt: &str) -> Result<String, Box<dyn Error>> {
        let mut child = Command::new("claude")
            .args(["-p", "--model", JUDGE_MODEL, "--output-format", "text"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .ok_or("claude stdin unavailable")?
            .write_all(prompt.as_bytes())?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(format!(
                "claude exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Content-addressed cache key over the local model + prompt.
    fn key(&self, prompt: &str) -> String {
        self.key_for(&self.model, prompt)
    }

    /// Cache key that also binds the pinned context window, so a prompt
    /// generated at one `num_ctx` is never served to a call that asked for a
    /// different one (a smaller window silently truncates — exactly what the
    /// full-context baseline must not do). `None` keeps the plain model+prompt
    /// key, leaving every existing default-window cache entry valid.
    fn key_ctx(&self, prompt: &str, num_ctx: Option<u64>) -> String {
        let Some(tokens) = num_ctx else {
            return self.key(prompt);
        };
        let mut hash = fnv1a(self.model.as_bytes());
        hash = fnv1a_continue(hash, b"\0num_ctx\0");
        hash = fnv1a_continue(hash, &tokens.to_le_bytes());
        hash = fnv1a_continue(hash, b"\0");
        hash = fnv1a_continue(hash, prompt.as_bytes());
        format!("{hash:016x}")
    }

    /// Content-addressed cache key over an explicit `model` + prompt, so callers
    /// using a different judging model get a disjoint cache namespace.
    #[allow(clippy::unused_self)]
    fn key_for(&self, model: &str, prompt: &str) -> String {
        let mut hash = fnv1a(model.as_bytes());
        hash = fnv1a_continue(hash, b"\0");
        hash = fnv1a_continue(hash, prompt.as_bytes());
        format!("{hash:016x}")
    }
}

/// Pull the `response` string plus the `prompt_eval_count`/`eval_count` usage
/// fields (absent on some Ollama versions — recorded as `None`, not guessed)
/// out of the JSON envelope.
fn parse_response(resp: ureq::Response) -> Result<GenReply, Box<dyn Error>> {
    let raw = resp.into_string()?;
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let text = value
        .get("response")
        .and_then(serde_json::Value::as_str)
        .ok_or("Ollama reply had no `response` field")?
        .trim()
        .to_string();
    let prompt_eval_count = value
        .get("prompt_eval_count")
        .and_then(serde_json::Value::as_u64);
    let eval_count = value.get("eval_count").and_then(serde_json::Value::as_u64);
    Ok(GenReply {
        text,
        prompt_eval_count,
        eval_count,
    })
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
