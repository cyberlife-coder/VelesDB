//! Concurrency contract for the per-fact write stripe
//! ([`crate::service::FactWriteLocks`]): the check-then-store window of
//! [`MemoryService::remember_hub`] must be serialized with
//! `wire_attributes`' merge on the same hub, or a racing bare store
//! overwrites attributes a concurrent call just merged.

use parking_lot::{Condvar, Mutex};
use serde_json::{json, Value};
use tempfile::TempDir;

use crate::embedder::{EmbedError, Embedder, HashEmbedder};
use crate::service::{MemoryService, Metadata};
use crate::storage::{FactStore, HUB_FIELD};
use crate::DEFAULT_DIMENSION;

/// A deterministic [`HashEmbedder`] whose FIRST embed, while armed, parks
/// the caller inside [`Embedder::embed`] until the test disarms it. That
/// reproduces the exact interleaving `remember_hub` lost attributes to: one
/// thread has read "hub absent" and is mid-embedding while a second thread
/// completes a full create+merge.
struct GateEmbedder {
    inner: HashEmbedder,
    gate: Mutex<Gate>,
    gate_cv: Condvar,
}

#[derive(Default)]
struct Gate {
    armed: bool,
    entered: bool,
}

impl Embedder for GateEmbedder {
    fn dimension(&self) -> usize {
        self.inner.dimension()
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        {
            let mut gate = self.gate.lock();
            if gate.armed && !gate.entered {
                gate.entered = true;
                self.gate_cv.notify_all();
                while gate.armed {
                    self.gate_cv.wait(&mut gate);
                }
            }
        }
        self.inner.embed(text)
    }
}

/// Two racing `remember_hub` creates for one new entity: the late thread's
/// bare store must not clobber the attributes the early thread merged
/// through `update_metadata` (what `wire_attributes` does under the stripe).
///
/// `GateEmbedder` parks thread A between its "hub absent" read and its
/// store; B then creates the hub and merges `age: 15` — the interleaving
/// that lost attributes before `remember_hub` took the hub's stripe.
#[test]
fn racing_hub_creates_never_clobber_a_merge_between_check_and_store() {
    let dir = TempDir::new().expect("tempdir");
    let embedder = GateEmbedder {
        inner: HashEmbedder::new(DEFAULT_DIMENSION),
        gate: Mutex::new(Gate {
            armed: true,
            entered: false,
        }),
        gate_cv: Condvar::new(),
    };
    let svc = MemoryService::open(dir.path(), embedder).expect("open store");

    std::thread::scope(|scope| {
        // A: sees "no hub", then stalls inside embed until released.
        let a = scope.spawn(|| svc.remember_hub("theo"));
        {
            let mut gate = svc.embedder.gate.lock();
            while !gate.entered {
                svc.embedder.gate_cv.wait(&mut gate);
            }
        }
        // B: the full create+merge lands while A is still mid-flight —
        // merge under the hub's stripe, exactly as wire_attributes does.
        let hub = svc.remember_hub("theo").expect("B remembers hub");
        let mut merged = Metadata::new();
        merged.insert("age".to_owned(), json!(15));
        {
            let _stripe = svc.fact_locks.for_fact(hub);
            svc.store.update_metadata(hub, &merged).expect("B merges");
        }
        // Release A: its stale "absent" read must not become a bare store
        // over the merged payload.
        {
            let mut gate = svc.embedder.gate.lock();
            gate.armed = false;
            svc.embedder.gate_cv.notify_all();
        }
        a.join().expect("A joins").expect("A remembers hub");

        let payload = svc.store.get_metadata(hub).unwrap().expect("hub stored");
        assert_eq!(
            payload.get(HUB_FIELD),
            Some(&Value::Bool(true)),
            "hub marker must survive the race"
        );
        assert_eq!(
            payload.get("age"),
            Some(&json!(15)),
            "A's delayed bare store must not overwrite B's merged attributes"
        );
    });
}

/// The serialized contract also pins the other half: once B's merge has
/// landed, A's later `remember_hub` must answer the existing id without
/// rewriting the payload — exercised here as a plain sequential double
/// resolve with a merge in between.
#[test]
fn hub_resolve_after_merge_returns_existing_without_rewriting() {
    let dir = TempDir::new().expect("tempdir");
    let svc =
        MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION)).expect("open store");

    let hub = svc.remember_hub("theo").expect("hub");
    let mut merged = Metadata::new();
    merged.insert("age".to_owned(), json!(15));
    svc.store.update_metadata(hub, &merged).expect("merge");

    let same = svc.remember_hub("theo").expect("re-resolve");
    assert_eq!(same, hub, "same topic must resolve to the same hub");
    let payload = svc.store.get_metadata(hub).unwrap().expect("hub stored");
    assert_eq!(payload.get("age"), Some(&json!(15)));
}
