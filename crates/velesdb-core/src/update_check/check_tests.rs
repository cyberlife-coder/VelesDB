use super::*;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::Instant;

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn payload() -> UpdateCheckPayload {
    UpdateCheckPayload::new(HASH.to_string(), "core", "server", false)
}

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

#[test]
fn test_payload_creation() {
    let payload = UpdateCheckPayload::new("abc123".to_string(), "core", "cli", true);

    assert!(!payload.version.is_empty());
    assert!(!payload.os.is_empty());
    assert!(!payload.arch.is_empty());
    assert_eq!(payload.instance_hash, "abc123");
    assert_eq!(payload.edition, "core");
    assert_eq!(payload.entry, "cli");
    assert!(payload.ci);
}

#[test]
fn test_payload_serialization() {
    let payload = UpdateCheckPayload::new("abc123".to_string(), "core", "server", true);
    let json = serde_json::to_string(&payload).expect("Failed to serialize");

    assert!(json.contains("\"os\""));
    assert!(json.contains("\"arch\""));
    assert!(json.contains("\"instance_hash\":\"abc123\""));
    assert!(json.contains("\"edition\":\"core\""));
    assert!(json.contains("\"entry\":\"server\""));
    assert!(json.contains("\"ci\":true"));
}

/// The payload is exactly the documented set of fields: a new one must
/// update `docs/guides/CONFIGURATION.md` and this list together.
#[test]
fn test_payload_carries_exactly_the_documented_fields() {
    let value = serde_json::to_value(payload()).expect("serialize");
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "arch",
            "ci",
            "edition",
            "entry",
            "instance_hash",
            "os",
            "version"
        ]
    );
    assert_eq!(value["ci"], serde_json::Value::Bool(false));
}

#[test]
fn test_disabled_run_builds_nothing_and_sends_nothing() {
    for cadence in [Cadence::Once, Cadence::Every(Duration::from_millis(1))] {
        let built = AtomicUsize::new(0);
        let sent = AtomicUsize::new(0);
        block_on(run(
            false,
            cadence,
            || {
                built.fetch_add(1, Ordering::SeqCst);
                payload()
            },
            |_| async {
                sent.fetch_add(1, Ordering::SeqCst);
            },
        ));
        assert_eq!(
            built.load(Ordering::SeqCst),
            0,
            "no id file is touched: {cadence:?}"
        );
        assert_eq!(
            sent.load(Ordering::SeqCst),
            0,
            "nothing is sent: {cadence:?}"
        );
    }
}

#[test]
fn test_one_shot_run_pings_once_and_returns() {
    let built = AtomicUsize::new(0);
    let sent = AtomicUsize::new(0);
    block_on(run(
        true,
        Cadence::Once,
        || {
            built.fetch_add(1, Ordering::SeqCst);
            payload()
        },
        |p| {
            assert_eq!(p.entry, "server");
            sent.fetch_add(1, Ordering::SeqCst);
            async {}
        },
    ));
    assert_eq!(built.load(Ordering::SeqCst), 1);
    assert_eq!(sent.load(Ordering::SeqCst), 1);
}

#[test]
fn test_heartbeat_repeats_with_one_payload_and_sleeps_between_pings() {
    const PINGS: u32 = 3;
    let interval = Duration::from_millis(40);
    let built = AtomicUsize::new(0);
    let sent = AtomicU32::new(0);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    let started = Instant::now();
    block_on(async {
        let heartbeat = run(
            true,
            Cadence::Every(interval),
            || {
                built.fetch_add(1, Ordering::SeqCst);
                payload()
            },
            |_| {
                // A loop that never sleeps never yields to the observer below,
                // so without this bound a missing sleep hangs instead of failing.
                assert!(
                    sent.fetch_add(1, Ordering::SeqCst) <= PINGS,
                    "heartbeat pinged again without sleeping"
                );
                let _ = tx.send(());
                async {}
            },
        );
        let observer = async {
            for _ in 0..PINGS {
                rx.recv().await.expect("heartbeat keeps pinging");
            }
        };
        tokio::select! {
            () = heartbeat => panic!("a heartbeat never returns"),
            () = observer => {}
        }
    });

    assert_eq!(built.load(Ordering::SeqCst), 1, "the payload is built once");
    assert!(
        started.elapsed() >= interval * (PINGS - 1),
        "{PINGS} pings need at least {} sleeps, took {:?}",
        PINGS - 1,
        started.elapsed()
    );
}

#[test]
fn test_heartbeat_interval_is_24_hours() {
    assert_eq!(HEARTBEAT_INTERVAL, Duration::from_secs(86_400));
}
