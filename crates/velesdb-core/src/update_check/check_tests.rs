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

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |key| {
        pairs
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_string())
    }
}

fn disabled() -> UpdateCheckConfig {
    UpdateCheckConfig {
        enabled: false,
        ..UpdateCheckConfig::default()
    }
}

const CADENCES: [Cadence; 2] = [Cadence::Once, Cadence::Every(Duration::from_millis(1))];

/// Every way of opting out reaches `drive`: it builds no payload (so no id
/// file is written) and sends nothing, for the start-up check and for the
/// heartbeat alike.
#[test]
fn test_every_opt_out_builds_nothing_and_sends_nothing() {
    struct OptOut {
        label: &'static str,
        config: UpdateCheckConfig,
        vars: &'static [(&'static str, &'static str)],
    }
    let cases = [
        OptOut {
            label: "config enabled = false",
            config: disabled(),
            vars: &[],
        },
        OptOut {
            label: "VELESDB_NO_UPDATE_CHECK=1",
            config: UpdateCheckConfig::default(),
            vars: &[("VELESDB_NO_UPDATE_CHECK", "1")],
        },
        OptOut {
            label: "VELESDB_UPDATE_CHECK=0",
            config: UpdateCheckConfig::default(),
            vars: &[("VELESDB_UPDATE_CHECK", "0")],
        },
        OptOut {
            label: "VELESDB_UPDATE_CHECK=off over a config that is on",
            config: UpdateCheckConfig::default(),
            vars: &[("VELESDB_UPDATE_CHECK", "off")],
        },
    ];
    for OptOut {
        label,
        config,
        vars,
    } in cases
    {
        for cadence in CADENCES {
            let built = AtomicUsize::new(0);
            let sent = AtomicUsize::new(0);
            block_on(drive(
                &config,
                env(vars),
                cadence,
                || {
                    built.fetch_add(1, Ordering::SeqCst);
                    payload()
                },
                |_| {
                    sent.fetch_add(1, Ordering::SeqCst);
                    async {}
                },
            ));
            assert_eq!(
                built.load(Ordering::SeqCst),
                0,
                "{label}, {cadence:?}: payload built"
            );
            assert_eq!(
                sent.load(Ordering::SeqCst),
                0,
                "{label}, {cadence:?}: ping sent"
            );
        }
    }
}

#[test]
fn test_one_shot_run_pings_once_and_returns() {
    let built = AtomicUsize::new(0);
    let sent = AtomicUsize::new(0);
    block_on(drive(
        &UpdateCheckConfig::default(),
        env(&[]),
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
fn test_enabling_variable_overrides_a_disabled_config() {
    let sent = AtomicUsize::new(0);
    block_on(drive(
        &disabled(),
        env(&[("VELESDB_UPDATE_CHECK", "1")]),
        Cadence::Once,
        payload,
        |_| {
            sent.fetch_add(1, Ordering::SeqCst);
            async {}
        },
    ));
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
        let config = UpdateCheckConfig::default();
        let heartbeat = drive(
            &config,
            env(&[]),
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
