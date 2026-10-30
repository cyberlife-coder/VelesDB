//! The `edge_store.bin` snapshot form of an [`EdgeStore`].
//!
//! The snapshot is postcard, which is not self-describing, and
//! `serde_json::Value` deserializes through `deserialize_any`, which postcard
//! cannot serve: written as a map, the properties of a single edge made the
//! whole snapshot unreadable. This form writes the store's `edges`,
//! `outgoing` and `incoming` maps in that order, each edge's fields in
//! [`GraphEdge`]'s order, with the properties as one JSON string, and an
//! empty map as the empty string. Postcard encodes an empty string and an
//! empty map with the same byte, so a snapshot of edges without properties
//! reads the same under both forms, and a store without properties writes
//! the same bytes it always did.
//!
//! The form is private to persistence: [`GraphEdge`]'s own serde
//! representation, which the REST API and the edge WAL carry as JSON, keeps
//! the properties as a map, and [`EdgeStore`] has no serde representation.

use std::collections::HashMap;

use serde::{de, ser, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::{EdgeStore, GraphEdge};

/// Encodes the persisted part of `store`.
///
/// # Errors
///
/// Returns a postcard error when an edge's properties nest deeper than
/// [`MAX_JSON_NESTING`](crate::validation::MAX_JSON_NESTING): the snapshot
/// never writes what it could not read back.
pub(in crate::collection::graph) fn to_bytes(
    store: &EdgeStore,
) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(&SnapshotRef {
        edges: EdgesRef(&store.edges),
        outgoing: &store.outgoing,
        incoming: &store.incoming,
    })
}

/// Decodes a snapshot into a store whose derived indexes (label table,
/// label indexes, CSR snapshot) are still empty.
///
/// # Errors
///
/// Returns a postcard error when the bytes are not a snapshot, including one
/// whose property strings are not JSON.
pub(in crate::collection::graph) fn from_bytes(bytes: &[u8]) -> Result<EdgeStore, postcard::Error> {
    let snapshot: Snapshot = postcard::from_bytes(bytes)?;
    Ok(EdgeStore {
        edges: snapshot.edges.0,
        outgoing: snapshot.outgoing,
        incoming: snapshot.incoming,
        ..EdgeStore::default()
    })
}

#[derive(Serialize)]
struct SnapshotRef<'a> {
    edges: EdgesRef<'a>,
    outgoing: &'a HashMap<u64, Vec<u64>>,
    incoming: &'a HashMap<u64, Vec<u64>>,
}

#[derive(Deserialize)]
struct Snapshot {
    edges: Edges,
    outgoing: HashMap<u64, Vec<u64>>,
    incoming: HashMap<u64, Vec<u64>>,
}

/// The edge map as the snapshot writes it.
struct EdgesRef<'a>(&'a HashMap<u64, GraphEdge>);

impl Serialize for EdgesRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Bounded by what this snapshot reads back (each property string is
        // its own JSON document), not by the stricter `MAX_VALUE_DEPTH` new
        // edges are held to: an edge a 7.x WAL replays must still snapshot.
        let readable = crate::validation::MAX_JSON_NESTING;
        if let Some((&key, _)) = self
            .0
            .iter()
            .find(|(_, e)| e.properties_nest_deeper_than(readable))
        {
            // postcard drops the message of a custom error: say it here.
            tracing::error!(
                edge = key,
                "edge properties nest deeper than the snapshot can read back; \
                 the snapshot is not written"
            );
            return Err(ser::Error::custom("edge properties nest too deep"));
        }
        serializer.collect_map(self.0.iter().map(|(&key, edge)| {
            let wire = WireEdgeRef {
                id: edge.id,
                source: edge.source,
                target: edge.target,
                label: &edge.label,
                properties: PropertiesJson(&edge.properties),
            };
            (key, wire)
        }))
    }
}

/// One edge as the snapshot writes it.
#[derive(Serialize)]
struct WireEdgeRef<'a> {
    id: u64,
    source: u64,
    target: u64,
    label: &'a str,
    properties: PropertiesJson<'a>,
}

/// An edge's properties written as one JSON string, the empty string when
/// there are none.
struct PropertiesJson<'a>(&'a HashMap<String, Value>);

impl Serialize for PropertiesJson<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.0.is_empty() {
            return serializer.serialize_str("");
        }
        let json = serde_json::to_string(self.0).map_err(ser::Error::custom)?;
        serializer.serialize_str(&json)
    }
}

/// The edge map as the snapshot reads it back.
struct Edges(HashMap<u64, GraphEdge>);

impl<'de> Deserialize<'de> for Edges {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(EdgesVisitor).map(Edges)
    }
}

/// One edge as the snapshot reads it back.
#[derive(Deserialize)]
struct WireEdge {
    id: u64,
    source: u64,
    target: u64,
    label: String,
    properties: String,
}

impl WireEdge {
    /// Rebuilds the edge field by field: the label was validated when the
    /// edge was first written, and is kept exactly as it was stored.
    fn into_edge(self) -> serde_json::Result<GraphEdge> {
        let properties = if self.properties.is_empty() {
            HashMap::new()
        } else {
            serde_json::from_str(&self.properties)?
        };
        Ok(GraphEdge {
            id: self.id,
            source: self.source,
            target: self.target,
            label: self.label,
            properties,
        })
    }
}

/// Builds the edge map entry by entry, so a load never holds the wire edges
/// and the decoded ones side by side.
struct EdgesVisitor;

impl<'de> de::Visitor<'de> for EdgesVisitor {
    type Value = HashMap<u64, GraphEdge>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a map of edge ids to edges")
    }

    fn visit_map<A: de::MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
        // Bounded like serde's own `HashMap` impl (at most 1 MiB up front):
        // a corrupt length prefix must not reserve gigabytes before the
        // first entry fails to decode.
        const MAX_PREALLOCATED: usize = 1024 * 1024 / std::mem::size_of::<(u64, GraphEdge)>();
        let capacity = access.size_hint().unwrap_or(0).min(MAX_PREALLOCATED);
        let mut edges = HashMap::with_capacity(capacity);
        while let Some((key, wire)) = access.next_entry::<u64, WireEdge>()? {
            let edge = wire.into_edge().map_err(|e| {
                // postcard drops the message of a custom error: say it here.
                tracing::error!(edge = key, error = %e, "edge snapshot property string is not JSON");
                de::Error::custom(e)
            })?;
            edges.insert(key, edge);
        }
        Ok(edges)
    }
}
