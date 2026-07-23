//! RED/GREEN proof that the HTTP transport's HTTPS-by-default listener is
//! real TLS termination, not a rubber-stamp: a client that does NOT trust
//! the freshly-generated local CA must fail the handshake (RED — proves
//! there's no `--insecure`-style bypass hiding underneath), and a client
//! configured to trust exactly that CA must complete a full
//! `remember`/`recall` MCP round trip over it (GREEN).
//!
//! Mirrors `tests/http_transport.rs`'s in-process pattern (bind
//! `127.0.0.1:0`, build the router directly, no subprocess) but adds a real
//! TLS accept loop (`velesdb_memory::http::serve_tls`) in front of it, fed
//! by CA/leaf material freshly generated per test
//! (`velesdb_memory::tls::ensure_tls_material`) into a scratch `TempDir` —
//! a fresh, randomly-keyed CA can never coincidentally already be trusted by
//! the machine running the test, so these tests are hermetic regardless of
//! whatever `scripts/install-memory-daemon.sh` may have done to this
//! machine's real login keychain in the past.

use std::net::SocketAddr;

use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use velesdb_memory::mcp::McpServer;
use velesdb_memory::tls::TlsMaterial;
use velesdb_memory::{DynEmbedder, HashEmbedder, MemoryService, DEFAULT_DIMENSION};

/// A running HTTPS transport for a test — the bound address, the accept
/// loop's task (drive it to completion via [`shutdown`]), the token that
/// stops it, and the two `TempDir`s (store + TLS material) kept alive for
/// the test's duration.
struct TlsTestServer {
    addr: SocketAddr,
    handle: JoinHandle<()>,
    ct: CancellationToken,
    ca_cert_path: std::path::PathBuf,
    _store_dir: tempfile::TempDir,
    _tls_dir: tempfile::TempDir,
}

async fn shutdown(server: TlsTestServer) {
    server.ct.cancel();
    server
        .handle
        .await
        .expect("http TLS server task must not panic");
}

/// Spin up the HTTPS transport on `127.0.0.1:0` (OS-assigned port) backed by
/// a fresh scratch store AND a fresh, freshly-generated local CA — see the
/// module docs for why a fresh-per-test CA keeps these tests hermetic.
async fn spawn_https_server() -> TlsTestServer {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(DEFAULT_DIMENSION));
    let service =
        MemoryService::open(store_dir.path(), embedder).expect("open scratch memory store");
    let server = McpServer::new(service);

    let tls_dir = tempfile::tempdir().expect("create scratch TLS material dir");
    let material: TlsMaterial = velesdb_memory::tls::ensure_tls_material(tls_dir.path())
        .expect("generate fresh CA + leaf certificate");
    let ca_cert_path = material.ca_cert_path.clone();
    let acceptor = velesdb_memory::tls::tls_acceptor_from_material(&material)
        .expect("build TLS acceptor from freshly generated material");

    let ct = CancellationToken::new();
    let app = velesdb_memory::http::router(server, ct.child_token());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral loopback port");
    let addr = listener.local_addr().expect("read bound local addr");

    let serve_ct = ct.clone();
    let handle = tokio::spawn(async move {
        velesdb_memory::http::serve_tls(app, listener, acceptor, serve_ct).await;
    });

    TlsTestServer {
        addr,
        handle,
        ct,
        ca_cert_path,
        _store_dir: store_dir,
        _tls_dir: tls_dir,
    }
}

/// A `reqwest::Client` that trusts only the ambient system/bundled roots —
/// deliberately NOT told about the test's freshly-generated CA. This is the
/// RED case's client: it must reject the server's certificate.
fn client_trusting_nothing_extra() -> reqwest::Client {
    reqwest::Client::builder()
        .build()
        .expect("build default reqwest client")
}

/// A `reqwest::Client` that additionally trusts `ca_cert_path`'s CA, on top
/// of the ambient system/bundled roots — the GREEN case's client.
fn client_trusting_ca(ca_cert_path: &std::path::Path) -> reqwest::Client {
    let ca_pem = std::fs::read(ca_cert_path).expect("read generated CA certificate");
    let ca_cert = reqwest::Certificate::from_pem(&ca_pem).expect("parse generated CA certificate");
    reqwest::Client::builder()
        .add_root_certificate(ca_cert)
        .build()
        .expect("build reqwest client trusting the generated CA")
}

/// Complete the MCP `initialize` handshake against `addr`'s `/mcp` endpoint
/// over HTTPS using `client`, and return the connected client.
async fn connect_https(
    addr: SocketAddr,
    client: reqwest::Client,
) -> RunningService<RoleClient, ClientInfo> {
    let transport = StreamableHttpClientTransport::with_client(
        client,
        StreamableHttpClientTransportConfig::with_uri(format!("https://{addr}/mcp")),
    );
    ClientInfo::default()
        .serve(transport)
        .await
        .expect("MCP initialize handshake over HTTPS")
}

fn as_args(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("expected a JSON object, got {other:?}"),
    }
}

async fn remember(client: &RunningService<RoleClient, ClientInfo>, fact: &str) -> String {
    let result = client
        .call_tool(
            CallToolRequestParams::new("remember").with_arguments(as_args(json!({ "fact": fact }))),
        )
        .await
        .expect("remember call over HTTPS");
    let structured = result
        .structured_content
        .expect("remember returns structured_content");
    structured["id_str"]
        .as_str()
        .expect("id_str is a string")
        .to_owned()
}

async fn recall_contains(
    client: &RunningService<RoleClient, ClientInfo>,
    query: &str,
    needle: &str,
) -> bool {
    let result = client
        .call_tool(
            CallToolRequestParams::new("recall").with_arguments(as_args(json!({
                "query": query,
                "limit": 50,
            }))),
        )
        .await
        .expect("recall call over HTTPS");
    let structured = result
        .structured_content
        .expect("recall returns structured_content");
    let memories = structured["memories"]
        .as_array()
        .expect("memories is an array");
    memories
        .iter()
        .any(|memory| memory["content"].as_str() == Some(needle))
}

/// RED: a client that does not trust the freshly-generated local CA must
/// fail to complete even a plain HTTPS GET against `/health` — proving the
/// server is doing real certificate-chain validation-worthy TLS, not
/// accepting any handshake.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_not_trusting_the_local_ca_is_rejected() {
    let server = spawn_https_server().await;

    let client = client_trusting_nothing_extra();
    let result = client
        .get(format!("https://{}/health", server.addr))
        .send()
        .await;

    assert!(
        result.is_err(),
        "a client that doesn't trust the freshly-generated CA must be rejected, got: {result:?}"
    );
    let err = result.unwrap_err();
    assert!(
        err.is_connect() || err.is_request(),
        "expected a TLS/connect-level failure (untrusted certificate), got: {err:?}"
    );

    shutdown(server).await;
}

/// GREEN: a client configured to trust exactly the generated CA completes
/// the TLS handshake and a full MCP `remember`/`recall` round trip over it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_trusting_the_local_ca_completes_remember_recall_roundtrip() {
    let server = spawn_https_server().await;

    let client = client_trusting_ca(&server.ca_cert_path);
    let health = client
        .get(format!("https://{}/health", server.addr))
        .send()
        .await
        .expect("HTTPS GET /health must succeed once the CA is trusted");
    assert!(health.status().is_success());

    let mcp_client = connect_https(server.addr, client).await;

    let fact = "HTTPS transport with a locally-trusted CA lets remember/recall round-trip";
    let id_str = remember(&mcp_client, fact).await;
    assert!(!id_str.is_empty(), "remember must return a non-empty id");
    assert!(
        recall_contains(&mcp_client, "locally-trusted CA", fact).await,
        "recall must surface the fact just remembered over HTTPS"
    );

    drop(mcp_client);
    shutdown(server).await;
}
