//! Building the configured embedding and extraction backends from the
//! environment: role endpoints, remote construction, and the reachability
//! warning at startup.

use velesdb_memory::DynEmbedder;

/// An embedder together with the identifier of the model behind it.
///
/// The model travels with the embedder because only the code that *built* it
/// knows its name: the [`velesdb_memory::Embedder`] trait exposes a dimension
/// and nothing else, deliberately — a trait implemented by callers should not
/// have to answer questions about a configuration it may not have. Carrying
/// the name here keeps [`velesdb_memory::embedding_provenance`] usable without
/// widening that trait for every implementor, in this crate and out of it.
pub(crate) struct ConfiguredEmbedder {
    pub(crate) embedder: DynEmbedder,
    /// As configured: `bge-m3`, `all-minilm`, or `hash` for the built-in.
    pub(crate) model: String,
}

/// Without the `extractor-http` feature there is no remote extraction backend in this
/// build, so there is nothing to be unreachable and no transport to ask with.
///
/// A no-op rather than a `cfg` at the call site, matching how
/// `build_remote_extractor` is paired a few lines below: the one arm that
/// reaches both is easier to read with the condition next to the reason than
/// wrapped around the code that uses it.
#[cfg(not(feature = "extractor-http"))]
pub(crate) fn warn_if_extraction_backend_is_unreachable(_backend: &str) {}

/// How long startup may spend asking whether the extraction backend is there.
///
/// Short on purpose: this runs before the daemon serves anything, and the
/// answer is worth having only if getting it costs nothing. A stalled server
/// is itself an answer, delivered by this timeout.
#[cfg(feature = "extractor-http")]
pub(crate) const EXTRACTION_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Say once, at startup, when the configured extraction backend cannot be
/// reached (#1751, decision D2).
///
/// A **signal, never a refusal.** Autograph degrading in flight is the correct
/// default — losing the enrichment beats losing the fact — and the arbitration
/// on #1751 forbids turning a successful `remember` into an error. What was
/// wrong is that the degradation was silent *forever*: an extractor broken by
/// a migration looked exactly like a product that does not build a graph.
/// Unreachable is also transient, so refusing to boot over it would replace a
/// silence with an outage.
///
/// Never falls back to another backend. A daemon that quietly answered with a
/// different engine than the one configured is the defect #1751's own gate
/// comment calls a contradiction the operator cannot resolve.
#[cfg(feature = "extractor-http")]
pub(crate) fn warn_if_extraction_backend_is_unreachable(backend: &str) {
    use velesdb_memory::reachability::{probe_openai, warning_line, Reachability};

    // `outline` is offline and deterministic: there is nothing to reach, and a
    // probe would invent a failure mode it does not have.
    if std::env::var_os("VELESDB_MEMORY_QUIET").is_some() {
        return;
    }
    let Some((url, model)) = extraction_endpoint_for_probe(backend) else {
        return;
    };
    let token = env_opt("VELESDB_MEMORY_EXTRACTOR_API_TOKEN");
    let outcome = probe_openai(&url, &model, token.as_deref(), EXTRACTION_PROBE_TIMEOUT);
    if outcome == Reachability::Reachable {
        return;
    }
    if let Some(line) = warning_line("extraction", &url, &model, &outcome) {
        eprintln!("{line}");
    }
}

/// The URL and model the extraction role will actually talk to, or `None` when
/// there is nothing to probe.
///
/// Resolves the same way the builders do — including Ollama's canonical local
/// default — because a probe of a different address than the one that will be
/// used answers a question nobody asked.
#[cfg(feature = "extractor-http")]
pub(crate) fn extraction_endpoint_for_probe(backend: &str) -> Option<(String, String)> {
    let endpoint = extractor_endpoint().ok()?;
    let url = match backend {
        "ollama" => Some(
            endpoint
                .url
                .unwrap_or_else(|| velesdb_memory::extract::DEFAULT_OLLAMA_URL.to_owned()),
        ),
        "openai" => endpoint.url,
        // An unwired name: `build_remote_extractor` has already refused it.
        _ => None,
    }?;
    Some((url, endpoint.model?))
}

/// Build the remote extraction backend named `backend`.
///
/// The `other` arm is not decoration. `select_extractor` is the single place
/// that knows which names exist, and it lives in the library while this
/// dispatch lives in the binary — so the two CAN drift. When they do, the
/// operator gets a message naming the gap instead of an Ollama client quietly
/// pointed at a server that speaks something else.
#[cfg(feature = "extractor-http")]
pub(crate) fn build_remote_extractor(
    backend: &str,
) -> Result<velesdb_memory::DynExtractor, Box<dyn std::error::Error>> {
    match backend {
        "ollama" => build_ollama_extractor(),
        "openai" => build_openai_extractor(),
        other => Err(unwired_backend("extraction", other).into()),
    }
}

/// Without the `extractor-http` feature there is no HTTP backend to build, whichever
/// one was asked for. The error names the offline alternative rather than only
/// what is missing: since #1734, `outline` is a real answer in **every** build,
/// so a user who only wanted a graph is one setting away instead of one
/// rebuild away.
#[cfg(not(feature = "extractor-http"))]
pub(crate) fn build_remote_extractor(
    backend: &str,
) -> Result<velesdb_memory::DynExtractor, Box<dyn std::error::Error>> {
    Err(format!(
        "VELESDB_MEMORY_EXTRACTOR={backend} needs a build with `--features extractor-http`; \
         for an offline deterministic graph with no rebuild, set \
         VELESDB_MEMORY_EXTRACTOR=outline instead"
    )
    .into())
}

// --- Where a remote backend's URL, model and credential come from -----------
//
// `RemoteEndpoint`/`role_auth` live in `velesdb_memory` (#1886): the daemon
// and the Python/Node bindings all resolve the embedding role's remote
// backend through the same library function now, instead of each reading (or,
// until #1886, simply not reading) the four `VELESDB_MEMORY_EMBEDDER*`
// variables on its own.

/// The embedding role's endpoint, honouring the legacy
/// `VELESDB_MEMORY_OLLAMA_*` aliases (C1). Thin wrapper over
/// [`velesdb_memory::embedder_env_endpoint`] that adds the one thing a
/// library must not do on a caller's behalf: print the alias-conflict notice
/// to stderr, gated on `VELESDB_MEMORY_QUIET` like every other startup notice.
///
/// # Errors
/// An `_API_TOKEN` that is set but empty.
#[cfg(feature = "embedder-http")]
pub(crate) fn embedder_endpoint(
) -> Result<velesdb_memory::RemoteEndpoint, Box<dyn std::error::Error>> {
    let (endpoint, notice) = velesdb_memory::embedder_env_endpoint()?;
    // Once, at startup, and only when the two genuinely disagree. Called from
    // `build_remote_embedder`, which runs exactly once per process.
    if let Some(notice) = notice {
        if std::env::var_os("VELESDB_MEMORY_QUIET").is_none() {
            eprintln!("{notice}");
        }
    }
    Ok(endpoint)
}

/// The extraction role's endpoint. No aliases to resolve: these variables were
/// role-named from the start, which is the naming the embedding side is being
/// brought in line with.
///
/// # Errors
/// An `_API_TOKEN` that is set but empty.
#[cfg(feature = "extractor-http")]
pub(crate) fn extractor_endpoint(
) -> Result<velesdb_memory::RemoteEndpoint, Box<dyn std::error::Error>> {
    Ok(velesdb_memory::RemoteEndpoint {
        url: env_opt("VELESDB_MEMORY_EXTRACTOR_URL"),
        model: env_opt("VELESDB_MEMORY_EXTRACTOR_MODEL"),
        auth: velesdb_memory::role_auth("VELESDB_MEMORY_EXTRACTOR_API_TOKEN")?,
    })
}

/// A variable's value, or `None` when it is unset.
#[cfg(any(feature = "embedder-http", feature = "extractor-http"))]
pub(crate) fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// A backend name the library accepts but this binary has no builder for.
///
/// Reachable only if `select_*` gains a name and the dispatch below is not
/// updated with it — the exact drift a wildcard arm used to hide.
#[cfg(any(feature = "embedder-http", feature = "extractor-http"))]
pub(crate) fn unwired_backend(role: &str, backend: &str) -> String {
    format!(
        "the {role} backend '{backend}' is accepted by velesdb-memory's selector but \
         the daemon has no builder wired for it — this is a bug in velesdb-memory, \
         not a configuration error; please report it quoting this message"
    )
}

/// Build the Ollama-backed extractor from `VELESDB_MEMORY_EXTRACTOR_URL`
/// (default local) and the required `VELESDB_MEMORY_EXTRACTOR_MODEL`.
#[cfg(feature = "extractor-http")]
pub(crate) fn build_ollama_extractor(
) -> Result<velesdb_memory::DynExtractor, Box<dyn std::error::Error>> {
    use std::sync::Arc;
    use velesdb_memory::extract::DEFAULT_OLLAMA_URL;
    use velesdb_memory::OllamaExtractor;

    let endpoint = extractor_endpoint()?;
    // A default URL is right HERE and wrong for `openai`: Ollama has one
    // canonical local address, an OpenAI-compatible server has none.
    let url = endpoint
        .url
        .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
    let model = endpoint.model.ok_or(
        "VELESDB_MEMORY_EXTRACTOR=ollama requires VELESDB_MEMORY_EXTRACTOR_MODEL \
         (e.g. qwen3.6:35b-mlx)",
    )?;
    Ok(Arc::new(OllamaExtractor::new(url, model)))
}

/// Build the OpenAI-compatible extractor from the extraction role's own
/// `VELESDB_MEMORY_EXTRACTOR_URL`, `_MODEL` and optional `_API_TOKEN`.
#[cfg(feature = "extractor-http")]
pub(crate) fn build_openai_extractor(
) -> Result<velesdb_memory::DynExtractor, Box<dyn std::error::Error>> {
    use std::sync::Arc;
    use velesdb_memory::OpenAiExtractor;

    let (url, model, auth) = extractor_endpoint()?.require("VELESDB_MEMORY_EXTRACTOR")?;
    Ok(Arc::new(OpenAiExtractor::new(url, model, auth)))
}

/// Select the embedding backend from `VELESDB_MEMORY_EMBEDDER`: `hash`
/// (default) is deterministic and fully offline; `ollama` / `openai` give real
/// on-device semantic recall. Both HTTP backends are compiled into the default
/// build, so the choice is an env-var switch, never a rebuild.
///
/// A thin read of the environment on top of
/// [`velesdb_memory::select_embedder`], mirroring what [`build_server`] does
/// for the extraction side. The choice itself lives in the library so the
/// daemon and the tests exercise the same function instead of a seam written
/// for the test.
///
/// `.ok()` maps an unset variable to `None`, which the library reads as "no
/// preference". A variable that IS set keeps its value — including an empty
/// one, which stays a caller error rather than collapsing into the default.
pub(crate) fn build_embedder() -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    let backend = std::env::var("VELESDB_MEMORY_EMBEDDER");
    build_embedder_selection(backend.as_deref().ok())
}

pub(crate) fn build_migration_target(
    backend: &str,
) -> Result<(DynEmbedder, String), velesdb_memory::MemoryError> {
    let configured = build_embedder_selection(Some(backend)).map_err(|error| {
        velesdb_memory::MemoryError::MigrationCapture(format!(
            "cannot configure migration target '{backend}': {error}"
        ))
    })?;
    Ok((configured.embedder, configured.model))
}

pub(crate) fn build_embedder_selection(
    backend: Option<&str>,
) -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    // The library message is transport-neutral; the daemon adds the name of the
    // thing the reader actually has to edit.
    let selection = velesdb_memory::select_embedder(backend)
        .map_err(|err| format!("VELESDB_MEMORY_EMBEDDER: {err}"))?;
    match selection {
        // Matched by name rather than by "ready implies hash": the startup
        // notice belongs to this one backend, and a future offline backend
        // must not inherit it silently.
        velesdb_memory::EmbedderSelection::Ready("hash", embedder) => {
            warn_hash_embedder_not_semantic();
            Ok(ConfiguredEmbedder {
                embedder,
                model: "hash".to_owned(),
            })
        }
        // For a backend that needs no configuration, the backend name IS the
        // model: there is no separate identifier to carry.
        velesdb_memory::EmbedderSelection::Ready(name, embedder) => Ok(ConfiguredEmbedder {
            embedder,
            model: name.to_owned(),
        }),
        // The name, not a wildcard — see `attach_extractor` for the defect
        // this shape removes on the extraction side. The embedding side never
        // had a second backend to get wrong, which is exactly why it was the
        // easier of the two to leave broken.
        velesdb_memory::EmbedderSelection::NeedsRemoteConfig(backend) => {
            build_remote_embedder(backend)
        }
    }
}

/// Build the remote embedding backend named `backend`.
///
/// Mirrors [`build_remote_extractor`], down to the `other` arm: the selector
/// lives in the library and this dispatch lives in the binary, so a name added
/// to one and not the other must say so rather than fall back to whichever
/// client happens to be first.
#[cfg(feature = "embedder-http")]
pub(crate) fn build_remote_embedder(
    backend: &str,
) -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    match backend {
        "ollama" => build_ollama_embedder(),
        "openai" => build_openai_embedder(),
        other => Err(unwired_backend("embedding", other).into()),
    }
}

/// Without the `embedder-http` feature this crate has no HTTP embedding backend
/// at all, whichever one was asked for.
#[cfg(not(feature = "embedder-http"))]
pub(crate) fn build_remote_embedder(
    backend: &str,
) -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    Err(format!(
        "the '{backend}' embedder requires building with `--features embedder-http` \
         (that feature carries the HTTP dependency for both remote embedding \
         backends); VELESDB_MEMORY_EMBEDDER=hash needs no rebuild"
    )
    .into())
}

/// Warn (on **stderr**, never stdout — that carries the MCP JSON-RPC stream)
/// that the default `hash` embedder is deterministic but **not semantic**:
/// `recall` matches on lexical/hash proximity, not meaning. This is the single
/// most common "why is recall bad?" surprise, so make the trade-off explicit
/// and point to the opt-in. Silence it for scripted/offline runs with
/// `VELESDB_MEMORY_QUIET=1`.
pub(crate) fn warn_hash_embedder_not_semantic() {
    if std::env::var_os("VELESDB_MEMORY_QUIET").is_some() {
        return;
    }
    eprintln!(
        "[velesdb-memory] {} For real semantic recall set \
         VELESDB_MEMORY_EMBEDDER=ollama or =openai \
         (no rebuild needed; see crates/velesdb-memory/README.md for the model \
         to pull). Set VELESDB_MEMORY_QUIET=1 to silence this notice.",
        velesdb_memory::HASH_EMBEDDER_NOTICE
    );
}

/// Build the Ollama-backed embedder, defaulting both the URL and the model —
/// unchanged behaviour, now reached through the role-named variables with the
/// `VELESDB_MEMORY_OLLAMA_*` pair kept working as aliases.
#[cfg(feature = "embedder-http")]
pub(crate) fn build_ollama_embedder() -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    use velesdb_memory::{OllamaEmbedder, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL};

    let endpoint = embedder_endpoint()?;
    let url = endpoint
        .url
        .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
    let model = endpoint
        .model
        .unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_owned());
    Ok(ConfiguredEmbedder {
        embedder: Box::new(OllamaEmbedder::new(&url, &model)?),
        model,
    })
}

/// Build the OpenAI-compatible embedder. Both the URL and the model are
/// required — see [`velesdb_memory::RemoteEndpoint::require`].
#[cfg(feature = "embedder-http")]
pub(crate) fn build_openai_embedder() -> Result<ConfiguredEmbedder, Box<dyn std::error::Error>> {
    use velesdb_memory::OpenAiEmbedder;

    let (url, model, auth) = embedder_endpoint()?.require("VELESDB_MEMORY_EMBEDDER")?;
    Ok(ConfiguredEmbedder {
        embedder: Box::new(OpenAiEmbedder::new(url, &model, auth)?),
        model,
    })
}
