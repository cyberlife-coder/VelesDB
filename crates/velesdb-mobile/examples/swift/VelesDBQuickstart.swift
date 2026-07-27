//
//  VelesDBQuickstart.swift
//  A complete walkthrough of the velesdb-mobile Swift binding.
//
//  Build the bindings first:
//      ./crates/velesdb-mobile/examples/generate_bindings.sh
//  then follow ./README.md to add velesdb_mobile.swift, velesdb_mobileFFI.h,
//  velesdb_mobileFFI.modulemap and the compiled library to your target.
//
//  If your framework is named something other than VelesDBMobile, change the
//  import below — or drop it entirely when the sources live in the same module
//  as this file.
//
//  IMPORTANT: nothing in this binding is async. Every call blocks the calling
//  thread. Run it off the main queue, as `runQuickstart(at:)` is designed to be.
//
//  Not compiled by CI: no workflow in the VelesDB repository builds Swift.
//  Read the generated velesdb_mobile.swift if a name here does not resolve.
//

import Foundation

#if canImport(VelesDBMobile)
import VelesDBMobile
#endif

// MARK: - Embeddings

/// Stand-in for a real embedding model.
///
/// The engine stores and searches vectors; producing them is the app's job.
/// On device that is Core ML, `NaturalLanguage`, or an ONNX runtime. This
/// deterministic hash keeps the example self-contained and its output stable.
func fakeEmbedding(_ text: String, dimension: Int = 4) -> [Float] {
    var values = [Float](repeating: 0, count: dimension)
    for (offset, scalar) in text.unicodeScalars.enumerated() {
        values[offset % dimension] += Float(scalar.value % 97) / 97.0
    }
    let norm = sqrt(values.reduce(0) { $0 + $1 * $1 })
    guard norm > 0 else { return values }
    return values.map { $0 / norm }
}

// MARK: - Quickstart

/// Runs the full walkthrough against a database at `path`.
///
/// - Parameter path: directory for the database; created if missing. On iOS use
///   the app's Application Support or Documents directory — never the bundle,
///   which is read-only.
func runQuickstart(at path: String) throws {
    // ---- 1. Open ------------------------------------------------------------
    // `open` is a NAMED constructor: a static method, not `VelesDatabase(path)`.
    // Looking for a default initializer is the most common first compile error.
    let db = try VelesDatabase.open(path: path)

    // ---- 2. Create a collection --------------------------------------------
    // Dimension and metric are immutable after creation. If the embedding model
    // changes, create a new collection and reindex — there is no ALTER path.
    // Use your model's dimension: 384 for all-MiniLM-L6-v2, 768 for MiniLM base.
    if try db.getCollection(name: "notes") == nil {
        try db.createCollection(name: "notes", dimension: 4, metric: .cosine)
    }
    guard let notes = try db.getCollection(name: "notes") else {
        fatalError("collection 'notes' vanished right after creation")
    }

    // ---- 3. Insert ----------------------------------------------------------
    // `payload` is a JSON *string*, not free text. Passing "hello" instead of
    // "{\"title\":\"hello\"}" fails with `Invalid JSON payload: ...` and commits
    // nothing.
    let documents: [(UInt64, String, String)] = [
        (1, "Groceries: milk, bread, coffee", "shopping"),
        (2, "Refactor the sync engine before the release", "work"),
        (3, "Book the dentist appointment", "health"),
        (4, "Coffee tasting notes: Ethiopian, floral", "shopping"),
    ]

    let points = try documents.map { id, text, category -> VelesPoint in
        let payload = try JSONSerialization.data(
            withJSONObject: ["text": text, "category": category]
        )
        return VelesPoint(
            id: id,
            vector: fakeEmbedding(text),
            payload: String(data: payload, encoding: .utf8)
        )
    }

    // upsertBatch is one engine round-trip for the whole slice; upsert(point:)
    // is the single-point form.
    try notes.upsertBatch(points: points)
    print("stored \(notes.count()) points, dimension \(notes.dimension())")

    // ---- 4. Search ----------------------------------------------------------
    let query = fakeEmbedding("what coffee did I like?")

    print("\nvector search:")
    for hit in try notes.search(vector: query, limit: 3) {
        print("  #\(hit.id)  score \(hit.score)  \(hit.payload ?? "<no payload>")")
    }

    // Quality profiles trade recall against latency. `.balanced` is the
    // production default; `.fast`, `.accurate` and `.perfect` move the
    // `ef_search` dial, and `.custom(ef:)` sets it outright.
    print("\nvector search, accurate profile:")
    for hit in try notes.searchWithQuality(vector: query, limit: 3, quality: .accurate) {
        print("  #\(hit.id)  score \(hit.score)")
    }

    // ---- 5. Filtered search -------------------------------------------------
    // The filter is a JSON string with the same grammar as the REST server:
    //   {"condition": {"type": "eq", "field": "...", "value": ...}}
    // Comparison types: eq, neq, gt, gte, lt, lte; "and" / "or" take a
    // `conditions` array and nest.
    let filterJson = #"{"condition": {"type": "eq", "field": "category", "value": "shopping"}}"#
    print("\nvector search restricted to category = shopping:")
    for hit in try notes.searchWithFilter(vector: query, limit: 3, filterJson: filterJson) {
        print("  #\(hit.id)  score \(hit.score)")
    }

    // ---- 6. Text and hybrid retrieval --------------------------------------
    // BM25 over the payload strings, no extra index to build.
    print("\nBM25 text search for \"coffee\":")
    for hit in try notes.textSearch(query: "coffee", limit: 3) {
        print("  #\(hit.id)  score \(hit.score)")
    }

    // Hybrid fuses both rankings. vectorWeight is the vector share:
    // 0.0 = text only, 1.0 = vector only.
    print("\nhybrid search (70% vector / 30% text):")
    for hit in try notes.hybridSearch(
        vector: query, textQuery: "coffee", limit: 3, vectorWeight: 0.7
    ) {
        print("  #\(hit.id)  score \(hit.score)")
    }

    // ---- 7. VelesQL ---------------------------------------------------------
    // Rows come back as JSON object strings — UniFFI cannot carry a dynamic
    // map across the FFI boundary, so each row is parsed on the Swift side.
    let result = try db.executeQuery(sql: "SELECT * FROM notes LIMIT 10", paramsJson: nil)
    print("\nVelesQL: \(result.message) (\(result.rowCount) rows)")
    for row in result.rows {
        guard let data = row.dataJson.data(using: .utf8) else { continue }
        let object = try JSONSerialization.jsonObject(with: data)
        print("  \(object)")
    }

    // ---- 8. Knowledge graph -------------------------------------------------
    // MobileGraphStore is a deliberate in-memory fork of core's graph engine:
    // RAM-only, no WAL, no on-disk payloads — hence the explicit save/load.
    let graph = MobileGraphStore()
    graph.addNode(node: MobileGraphNode(
        id: 1, label: "Person", propertiesJson: #"{"name":"Ada"}"#, vector: nil
    ))
    graph.addNode(node: MobileGraphNode(
        id: 2, label: "Person", propertiesJson: #"{"name":"Grace"}"#, vector: nil
    ))
    graph.addNode(node: MobileGraphNode(
        id: 3, label: "Project", propertiesJson: #"{"name":"Sync"}"#, vector: nil
    ))
    // addEdge throws when an endpoint has no stored node.
    try graph.addEdge(edge: MobileGraphEdge(
        id: 100, source: 1, target: 2, label: "KNOWS", propertiesJson: nil
    ))
    try graph.addEdge(edge: MobileGraphEdge(
        id: 101, source: 2, target: 3, label: "WORKS_ON", propertiesJson: nil
    ))

    print("\ngraph: \(graph.nodeCount()) nodes, \(graph.edgeCount()) edges")
    print("BFS from node 1, depth <= 2:")
    for step in graph.bfsTraverse(sourceId: 1, maxDepth: 2, limit: 100) {
        print("  node \(step.nodeId) at depth \(step.depth) via \(step.path)")
    }
    // The source itself is never emitted (depth 0 is skipped) and every node is
    // visited at most once, so a diamond does not produce duplicates.

    // Persist it explicitly — nothing about this store is automatic.
    let graphPath = (path as NSString).appendingPathComponent("graph.bin")
    try graph.save(path: graphPath)
    let reloaded = try MobileGraphStore.load(path: graphPath)
    print("reloaded graph: \(reloaded.nodeCount()) nodes")

    // ---- 9. Agent memory ----------------------------------------------------
    // VelesSemanticMemory keeps facts with their embeddings and reads the text
    // back out of the payload, so content survives a database reload.
    // Semantic only: episodic and procedural memory are not exposed on mobile.
    let memory = try VelesSemanticMemory(db: db, dimension: 4)
    try memory.store(id: 1, content: "The user prefers dark roast coffee.",
                     embedding: fakeEmbedding("The user prefers dark roast coffee."))
    try memory.store(id: 2, content: "The user is allergic to penicillin.",
                     embedding: fakeEmbedding("The user is allergic to penicillin."))

    print("\nsemantic memory (\(try memory.len()) facts):")
    for hit in try memory.query(embedding: fakeEmbedding("coffee preference"), topK: 2) {
        print("  #\(hit.id)  score \(hit.score)  \(hit.content)")
    }

    // ---- 10. The errors you will actually hit -------------------------------
    // A vector whose length differs from the collection's dimension.
    do {
        _ = try notes.search(vector: [1.0, 0.0], limit: 1)
        print("\nUNEXPECTED: a 2-dimensional query should not have been accepted")
    } catch {
        // The generated enum is `VelesError` with `Database`, `Collection` and
        // `DimensionMismatch` variants. Case spelling follows whatever the
        // generated velesdb_mobile.swift uses, so match on it there rather than
        // guessing here; `error` is always printable.
        print("\ndimension mismatch rejected as expected: \(error)")
    }

    // A payload that is not JSON. Nothing is committed when this fires.
    do {
        try notes.upsert(point: VelesPoint(
            id: 99, vector: fakeEmbedding("bad"), payload: "not json"
        ))
        print("UNEXPECTED: a non-JSON payload should not have been accepted")
    } catch {
        print("invalid payload rejected as expected: \(error)")
    }

    // streamInsert before enableStreaming.
    do {
        _ = try notes.streamInsert(points: [
            VelesPoint(id: 98, vector: fakeEmbedding("stream"), payload: nil)
        ])
        print("UNEXPECTED: streaming was never enabled")
    } catch {
        print("stream insert without enableStreaming rejected as expected: \(error)")
    }
}

// MARK: - Entry point

/// Call this from `application(_:didFinishLaunchingWithOptions:)`, a SwiftUI
/// `.task`, or a unit test. It hops off the main queue because every binding
/// call is blocking and a search on the main thread freezes the UI.
func runQuickstartOffMainThread() {
    DispatchQueue.global(qos: .userInitiated).async {
        let path = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("velesdb")
            .path
        try? FileManager.default.createDirectory(
            atPath: path, withIntermediateDirectories: true
        )

        do {
            try runQuickstart(at: path)
        } catch {
            print("velesdb quickstart failed: \(error)")
        }
    }
}
