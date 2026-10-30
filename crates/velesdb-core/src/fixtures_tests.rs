use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// Creates a test collection with the given dimension and cosine metric.
///
/// Returns the `TempDir` guard (must be kept alive for the collection's
/// lifetime) and the newly created `Collection`.
pub fn setup_collection(dim: usize) -> (TempDir, Collection) {
    let dir = tempfile::tempdir().expect("test: tempdir creation");
    let col = Collection::create(PathBuf::from(dir.path()), dim, DistanceMetric::Cosine)
        .expect("test: collection creation");
    (dir, col)
}

/// Creates a simple test point with no payload or sparse vectors.
#[allow(dead_code)] // Available for future test adoption.
pub fn make_point(id: u64, vector: Vec<f32>) -> Point {
    Point {
        id,
        vector,
        payload: None,
        sparse_vectors: None,
    }
}

/// Creates a test point with a JSON payload.
pub fn make_point_with_payload(id: u64, vector: Vec<f32>, payload: serde_json::Value) -> Point {
    Point {
        id,
        vector,
        payload: Some(payload),
        sparse_vectors: None,
    }
}

// ============================================================================
// Minimal warn-capturing tracing subscriber (no extra dev-dependency).
// ============================================================================

/// Captures WARN-and-worse events as flat `field=value` strings.
#[derive(Default)]
struct WarnSink {
    events: Mutex<Vec<String>>,
}

struct FlattenVisitor<'a>(&'a mut String);

impl tracing::field::Visit for FlattenVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let _ = write!(self.0, "{}={:?} ", field.name(), value);
    }
}

struct WarnCapture {
    sink: Arc<WarnSink>,
}

impl tracing::Subscriber for WarnCapture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        // In `tracing`, more severe levels compare LESS (ERROR < WARN < INFO).
        *metadata.level() <= tracing::Level::WARN
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut line = String::new();
        event.record(&mut FlattenVisitor(&mut line));
        self.sink
            .events
            .lock()
            .expect("test: warn sink lock")
            .push(line);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// Runs `f` with a warn-capturing subscriber installed (thread-local) and
/// returns the captured WARN+ events. Shared by any test that needs to prove
/// a `tracing::warn!` actually fires — a `tracing` warning is not otherwise
/// observable from a test.
#[allow(dead_code)] // Available for future test adoption.
pub fn capture_warns<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    let sink = Arc::new(WarnSink::default());
    let subscriber = WarnCapture { sink: sink.clone() };
    let out = tracing::subscriber::with_default(subscriber, f);
    let events = sink.events.lock().expect("test: warn sink lock").clone();
    (out, events)
}
