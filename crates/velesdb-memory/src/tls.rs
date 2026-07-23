//! Locally-generated TLS material for the streamable-HTTP transport.
//!
//! Claude Desktop's "Add custom connector" UI refuses any URL that isn't
//! `https://`, even for `127.0.0.1` — so the HTTP transport (see
//! [`crate::http`]) now terminates TLS itself by default, with no external
//! dependency (no shelled-out `mkcert`/`openssl`, no reverse proxy the user
//! has to install and keep running).
//!
//! The approach: a self-signed root CA is generated ONCE and cached on disk
//! (see [`tls_dir_from_env`]); it signs short-lived `localhost`/`127.0.0.1`
//! leaf certificates that are silently re-issued on every daemon start. A
//! client (browser, `curl`, Claude Desktop) that has been told to trust the
//! CA once — `scripts/install-memory-daemon.sh` adds it to the macOS login
//! keychain — then trusts every future leaf cert it signs with no further
//! action, even across daemon restarts and leaf renewals, because the CA's
//! own key never changes. [`ensure_tls_material`] is what enforces that:
//! it reloads an existing CA from disk instead of regenerating one whenever
//! both `ca-cert.pem` and `ca-key.pem` are already present.
//!
//! # Why manual `tokio_rustls` instead of `axum-server`
//! `crates/velesdb-server` already terminates TLS this way (its own
//! `src/tls.rs` builds a [`tokio_rustls::TlsAcceptor`]; its `src/main.rs`
//! runs the accept loop). Reusing that exact pattern here — a
//! [`tokio_rustls::TlsAcceptor`] wrapping each accepted `TcpStream`, served
//! through `hyper_util`'s auto h1/h2 connection builder — rather than
//! adding `axum-server` as a second, independent TLS dependency, keeps the
//! workspace's TLS story to one crate family (`rustls`/`tokio-rustls`/
//! `rustls-pemfile`, already pinned in `Cargo.lock`) and avoids
//! re-litigating `axum-server`'s own rustls-version compatibility against
//! `axum 0.8` for what both crates ultimately do the same way underneath.
//! [`crate::http::serve_tls`] is the accept-loop half of this; this module
//! is only the certificate material.
//!
//! # An explicit `CryptoProvider`, not the process-wide default
//! [`tls_acceptor_from_material`] builds its [`rustls::ServerConfig`] with
//! `ServerConfig::builder_with_provider(..)` and an explicit
//! `rustls::crypto::ring::default_provider()`, rather than the ambient
//! `ServerConfig::builder()` (which resolves the process-wide default
//! `CryptoProvider`). That default is genuinely ambiguous here: the
//! `reqwest` dev-dependency (pulled in by rmcp's streamable-HTTP client
//! transport, used by `tests/http_tls.rs`) links `rustls` with the
//! `aws-lc-rs` backend, while this module always wants `ring` (matching
//! `velesdb-server`'s choice) — and both backends end up linked into the
//! SAME test binary. Relying on "whichever one calls
//! `CryptoProvider::install_default()` first" would make the test suite's
//! behavior depend on initialization order between unrelated crates.
//! Passing the provider explicitly sidesteps that entirely.

use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, SanType,
};
use time::{Duration as TimeDuration, OffsetDateTime};

/// File names inside the TLS material directory (see [`tls_dir_from_env`]).
/// `ca-cert.pem` is public (safe to hand to `security add-trusted-cert`,
/// safe to be world-readable); the two `*-key.pem` files are private and are
/// always written with `0600` permissions (see [`set_permissions`]).
pub const CA_CERT_FILE: &str = "ca-cert.pem";
const CA_KEY_FILE: &str = "ca-key.pem";
const LEAF_CERT_FILE: &str = "leaf-cert.pem";
const LEAF_KEY_FILE: &str = "leaf-key.pem";

/// Leaf certs are silently re-issued on every daemon start (see module
/// docs), so a short lifetime costs nothing in UX while shrinking the
/// exposure window of a leaf private key that — unlike the CA's — is
/// rewritten to disk on every run. 30 days is generous headroom for a
/// daemon left running unattended over a long weekend without a restart.
const LEAF_CERT_LIFETIME_DAYS: i64 = 30;

/// The CA is meant to be trusted once and left alone indefinitely (that's
/// the entire point of caching it — see module docs), so it gets a long,
/// "effectively permanent for this use case" lifetime rather than the
/// leaf's short one.
const CA_CERT_LIFETIME_DAYS: i64 = 3650;

/// Backdate `not_before` by this much on every generated certificate, to
/// absorb clock skew between "when this process thinks it is" and "when the
/// TLS client checking the cert's validity window thinks it is" — cheap
/// insurance against a leap-second/NTP hiccup rejecting a cert the instant
/// it's minted.
const NOT_BEFORE_SLACK_HOURS: i64 = 1;

/// Errors from generating or loading local TLS material. Deliberately
/// distinct from [`crate::error::MemoryError`] — these are infra/filesystem
/// concerns, not store/domain ones.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("failed to create TLS material directory {path}: {source}")]
    CreateDir { path: PathBuf, source: io::Error },
    #[error("failed to read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("failed to write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("failed to set permissions on {path}: {source}")]
    Permissions { path: PathBuf, source: io::Error },
    #[error("certificate generation failed: {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("invalid generated TLS certificate/key material: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("failed to parse generated leaf certificate PEM: {0}")]
    PemCert(io::Error),
    #[error("failed to parse generated leaf private key PEM: {0}")]
    PemKey(io::Error),
    #[error("generated leaf material contained no private key")]
    NoPrivateKey,
}

/// A leaf certificate + private key (PEM), ready to serve, plus the CA's
/// public certificate path (for callers — the installer, startup banner —
/// that need to point a user or `security add-trusted-cert` at it).
pub struct TlsMaterial {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    pub ca_cert_path: PathBuf,
}

/// Default TLS material directory when `VELESDB_MEMORY_TLS_DIR` is unset: a
/// sibling of the default store (`~/.velesdb-memory`), deliberately NOT
/// nested inside it. The store and the CA have independent lifecycles — a
/// user who wipes `~/.velesdb-memory` to reset their memory shouldn't also
/// silently invalidate a CA their OS (or a client like Claude Desktop) has
/// already been told to trust, and vice versa. Mirrors `default_store_path`
/// in `src/main.rs`: a stable home-based path, since an MCP client launches
/// this process with an unpredictable working directory.
#[must_use]
pub fn default_tls_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty());
    match home {
        Some(home) => Path::new(&home).join(".velesdb-memory-tls"),
        None => PathBuf::from("./velesdb-memory-tls"),
    }
}

/// Resolve the TLS material directory from `VELESDB_MEMORY_TLS_DIR`, falling
/// back to [`default_tls_dir`] when unset.
#[must_use]
pub fn tls_dir_from_env() -> PathBuf {
    std::env::var_os("VELESDB_MEMORY_TLS_DIR").map_or_else(default_tls_dir, PathBuf::from)
}

/// Ensure a local CA exists at `dir` (generating one on first use, reusing
/// it otherwise — see module docs), then issue a fresh short-lived leaf
/// certificate signed by it. Creates `dir` (and sets its permissions) if
/// needed.
///
/// # Errors
/// Returns [`TlsError`] if `dir` cannot be created/written to, or if
/// certificate generation/parsing fails.
pub fn ensure_tls_material(dir: &Path) -> Result<TlsMaterial, TlsError> {
    create_private_dir(dir)?;
    let issuer = ensure_ca(dir)?;
    let (cert_pem, key_pem) = issue_leaf_cert(dir, &issuer)?;
    Ok(TlsMaterial {
        cert_pem,
        key_pem,
        ca_cert_path: dir.join(CA_CERT_FILE),
    })
}

/// Build a [`tokio_rustls::TlsAcceptor`] from generated [`TlsMaterial`],
/// using an explicit `ring` [`rustls::crypto::CryptoProvider`] — see the
/// module docs for why that's explicit rather than ambient.
///
/// # Errors
/// Returns [`TlsError`] if the PEM material fails to parse, or if `rustls`
/// rejects the resulting configuration.
pub fn tls_acceptor_from_material(
    material: &TlsMaterial,
) -> Result<tokio_rustls::TlsAcceptor, TlsError> {
    let certs: Vec<_> = rustls_pemfile::certs(&mut &material.cert_pem[..])
        .collect::<Result<Vec<_>, _>>()
        .map_err(TlsError::PemCert)?;
    let key = rustls_pemfile::private_key(&mut &material.key_pem[..])
        .map_err(TlsError::PemKey)?
        .ok_or(TlsError::NoPrivateKey)?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

/// Create `dir` if missing and lock it down to the owning user (`0700`) —
/// belt-and-suspenders alongside the per-file `0600` on the two private key
/// files: even a directory listing of a world-readable dir would leak that
/// a CA key exists here and its filename, so the directory itself is
/// private too.
fn create_private_dir(dir: &Path) -> Result<(), TlsError> {
    fs::create_dir_all(dir).map_err(|source| TlsError::CreateDir {
        path: dir.to_owned(),
        source,
    })?;
    set_permissions(dir, 0o700)
}

/// Load the CA from `dir` if both its cert and key are already present
/// (never regenerating one that exists — see module docs for why that
/// matters), otherwise generate a fresh self-signed CA and persist it.
/// Either way, returns an [`Issuer`] ready to sign a leaf certificate.
fn ensure_ca(dir: &Path) -> Result<Issuer<'static, KeyPair>, TlsError> {
    let ca_cert_path = dir.join(CA_CERT_FILE);
    let ca_key_path = dir.join(CA_KEY_FILE);

    if ca_cert_path.exists() && ca_key_path.exists() {
        let ca_cert_pem = read_to_string(&ca_cert_path)?;
        let ca_key_pem = read_to_string(&ca_key_path)?;
        let key_pair = KeyPair::from_pem(&ca_key_pem)?;
        let issuer = Issuer::from_ca_cert_pem(&ca_cert_pem, key_pair)?;
        return Ok(issuer);
    }

    let key_pair = KeyPair::generate()?;
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.not_before = OffsetDateTime::now_utc() - TimeDuration::hours(NOT_BEFORE_SLACK_HOURS);
    params.not_after = OffsetDateTime::now_utc() + TimeDuration::days(CA_CERT_LIFETIME_DAYS);
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "VelesDB Memory Local CA");
    dn.push(DnType::OrganizationName, "VelesDB Memory (local)");
    params.distinguished_name = dn;

    let ca_cert = params.self_signed(&key_pair)?;

    write_file(&ca_cert_path, ca_cert.pem().as_bytes(), 0o644)?;
    write_file(&ca_key_path, key_pair.serialize_pem().as_bytes(), 0o600)?;

    Ok(Issuer::new(params, key_pair))
}

/// Issue a fresh leaf certificate for `localhost`/`127.0.0.1`/`::1`, signed
/// by `issuer`, and persist it to `dir` (rewriting any previous leaf —
/// silent renewal, no new trust required, see module docs). Returns the PEM
/// bytes directly so the caller doesn't have to re-read them from disk.
fn issue_leaf_cert(
    dir: &Path,
    issuer: &Issuer<'_, KeyPair>,
) -> Result<(Vec<u8>, Vec<u8>), TlsError> {
    let leaf_key = KeyPair::generate()?;
    let mut params = CertificateParams::new(vec!["localhost".to_owned()])?;
    params.subject_alt_names = vec![
        SanType::DnsName("localhost".try_into()?),
        SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        SanType::IpAddress(IpAddr::V6(Ipv6Addr::LOCALHOST)),
    ];
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "localhost");
    params.distinguished_name = dn;
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.not_before = OffsetDateTime::now_utc() - TimeDuration::hours(NOT_BEFORE_SLACK_HOURS);
    params.not_after = OffsetDateTime::now_utc() + TimeDuration::days(LEAF_CERT_LIFETIME_DAYS);

    let cert = params.signed_by(&leaf_key, issuer)?;
    let cert_pem = cert.pem().into_bytes();
    let key_pem = leaf_key.serialize_pem().into_bytes();

    write_file(&dir.join(LEAF_CERT_FILE), &cert_pem, 0o644)?;
    write_file(&dir.join(LEAF_KEY_FILE), &key_pem, 0o600)?;

    Ok((cert_pem, key_pem))
}

fn read_to_string(path: &Path) -> Result<String, TlsError> {
    fs::read_to_string(path).map_err(|source| TlsError::Read {
        path: path.to_owned(),
        source,
    })
}

fn write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), TlsError> {
    fs::write(path, bytes).map_err(|source| TlsError::Write {
        path: path.to_owned(),
        source,
    })?;
    set_permissions(path, mode)
}

/// Set Unix permission bits on `path`. The private key files (`0600`) are
/// the real security boundary here — see the `# Decisions` note in this
/// crate's task history: a private key readable by other local users would
/// let them impersonate this daemon's TLS identity to any client that
/// trusts the CA.
#[cfg(unix)]
fn set_permissions(path: &Path, mode: u32) -> Result<(), TlsError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| {
        TlsError::Permissions {
            path: path.to_owned(),
            source,
        }
    })
}

/// No POSIX mode bits on Windows. The files already live under the current
/// user's profile directory (`default_tls_dir`'s `USERPROFILE` fallback),
/// which is private-by-default under the OS's own ACLs — a deliberate
/// no-op rather than a partial/misleading permissions story, same pattern
/// as `spawn_orphan_watchdog` in `src/main.rs` being Unix-only.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn set_permissions(_path: &Path, _mode: u32) -> Result<(), TlsError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_a_ca_and_leaf_cert_on_first_use() {
        let dir = tempfile::tempdir().expect("create scratch dir");
        let material = ensure_tls_material(dir.path()).expect("generate TLS material");

        assert!(!material.cert_pem.is_empty());
        assert!(!material.key_pem.is_empty());
        assert!(material.ca_cert_path.exists());
        assert!(dir.path().join(CA_KEY_FILE).exists());
    }

    #[test]
    fn reuses_the_same_ca_across_calls() {
        let dir = tempfile::tempdir().expect("create scratch dir");
        let first = ensure_tls_material(dir.path()).expect("first generation");
        let ca_pem_first =
            std::fs::read_to_string(&first.ca_cert_path).expect("read CA cert after first run");

        let second = ensure_tls_material(dir.path()).expect("second generation");
        let ca_pem_second =
            std::fs::read_to_string(&second.ca_cert_path).expect("read CA cert after second run");

        assert_eq!(
            ca_pem_first, ca_pem_second,
            "the CA must never be regenerated once it exists on disk"
        );
    }

    #[test]
    fn leaf_cert_is_re_issued_on_every_call() {
        let dir = tempfile::tempdir().expect("create scratch dir");
        let first = ensure_tls_material(dir.path()).expect("first generation");
        let second = ensure_tls_material(dir.path()).expect("second generation");

        assert_ne!(
            first.cert_pem, second.cert_pem,
            "the leaf certificate is expected to be freshly re-issued (renewed) on every start"
        );
    }

    #[test]
    fn builds_a_tls_acceptor_from_generated_material() {
        let dir = tempfile::tempdir().expect("create scratch dir");
        let material = ensure_tls_material(dir.path()).expect("generate TLS material");
        tls_acceptor_from_material(&material).expect("build TLS acceptor from valid material");
    }

    #[cfg(unix)]
    #[test]
    fn private_key_files_are_not_group_or_world_readable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("create scratch dir");
        ensure_tls_material(dir.path()).expect("generate TLS material");

        for key_file in [CA_KEY_FILE, LEAF_KEY_FILE] {
            let path = dir.path().join(key_file);
            let mode = std::fs::metadata(&path)
                .unwrap_or_else(|_| panic!("stat {key_file}"))
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o077,
                0,
                "{key_file} must not be group/world readable or writable (mode {mode:o})"
            );
        }
    }
}
