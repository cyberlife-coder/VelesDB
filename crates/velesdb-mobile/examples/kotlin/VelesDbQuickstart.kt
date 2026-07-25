/*
 * VelesDbQuickstart.kt
 * A complete walkthrough of the velesdb-mobile Kotlin binding.
 *
 * Build the bindings first:
 *     ./crates/velesdb-mobile/examples/generate_bindings.sh
 * then follow ./README.md to add the generated
 * uniffi/velesdb_mobile/velesdb_mobile.kt, JNA, and the per-ABI .so files to
 * your Gradle module.
 *
 * IMPORTANT: nothing in this binding is suspending. Every call blocks the
 * calling thread. Run it on Dispatchers.IO, as `runQuickstartAsync` does.
 *
 * Not compiled by CI: no workflow in the VelesDB repository builds Kotlin.
 * Read the generated velesdb_mobile.kt if a name here does not resolve.
 */

package com.example.velesdbquickstart

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.json.JSONObject
import uniffi.velesdb_mobile.DistanceMetric
import uniffi.velesdb_mobile.MobileGraphEdge
import uniffi.velesdb_mobile.MobileGraphNode
import uniffi.velesdb_mobile.MobileGraphStore
import uniffi.velesdb_mobile.SearchQuality
import uniffi.velesdb_mobile.VelesDatabase
import uniffi.velesdb_mobile.VelesPoint
import uniffi.velesdb_mobile.VelesSemanticMemory
import java.io.File
import kotlin.math.sqrt

// ---------------------------------------------------------------------------
// Embeddings
// ---------------------------------------------------------------------------

/**
 * Stand-in for a real embedding model.
 *
 * The engine stores and searches vectors; producing them is the app's job. On
 * device that is ML Kit, TensorFlow Lite, or an ONNX runtime. This
 * deterministic hash keeps the example self-contained and its output stable.
 */
fun fakeEmbedding(text: String, dimension: Int = 4): List<Float> {
    val values = FloatArray(dimension)
    text.forEachIndexed { index, ch ->
        values[index % dimension] += (ch.code % 97) / 97.0f
    }
    val norm = sqrt(values.fold(0.0f) { acc, v -> acc + v * v })
    if (norm == 0.0f) return values.toList()
    return values.map { it / norm }
}

// ---------------------------------------------------------------------------
// Quickstart
// ---------------------------------------------------------------------------

/**
 * Runs the full walkthrough against a database at [path].
 *
 * @param path directory for the database; created if missing. On Android use
 *   `context.filesDir` or `context.noBackupFilesDir` — never the APK assets,
 *   which are read-only.
 */
fun runQuickstart(path: String) {
    // ---- 1. Open ----------------------------------------------------------
    // `open` is a NAMED constructor: a companion factory, not `VelesDatabase(path)`.
    // Looking for a normal constructor is the most common first compile error.
    val db = VelesDatabase.open(path)

    // ---- 2. Create a collection -------------------------------------------
    // Dimension and metric are immutable after creation. If the embedding model
    // changes, create a new collection and reindex — there is no ALTER path.
    // Use your model's dimension: 384 for all-MiniLM-L6-v2, 768 for MiniLM base.
    if (db.getCollection("notes") == null) {
        db.createCollection("notes", 4u, DistanceMetric.COSINE)
    }
    val notes = db.getCollection("notes")
        ?: error("collection 'notes' vanished right after creation")

    // ---- 3. Insert --------------------------------------------------------
    // `payload` is a JSON *string*, not free text. Passing "hello" instead of
    // {"title":"hello"} fails with `Invalid JSON payload: ...` and commits
    // nothing.
    val documents = listOf(
        Triple(1uL, "Groceries: milk, bread, coffee", "shopping"),
        Triple(2uL, "Refactor the sync engine before the release", "work"),
        Triple(3uL, "Book the dentist appointment", "health"),
        Triple(4uL, "Coffee tasting notes: Ethiopian, floral", "shopping"),
    )

    val points = documents.map { (id, text, category) ->
        val payload = JSONObject()
            .put("text", text)
            .put("category", category)
            .toString()
        VelesPoint(id = id, vector = fakeEmbedding(text), payload = payload)
    }

    // upsertBatch is one engine round-trip for the whole list; upsert(point)
    // is the single-point form.
    notes.upsertBatch(points)
    println("stored ${notes.count()} points, dimension ${notes.dimension()}")

    // ---- 4. Search --------------------------------------------------------
    val query = fakeEmbedding("what coffee did I like?")

    println("\nvector search:")
    notes.search(query, 3u).forEach { hit ->
        println("  #${hit.id}  score ${hit.score}  ${hit.payload ?: "<no payload>"}")
    }

    // Quality profiles trade recall against latency. BALANCED is the production
    // default; FAST, ACCURATE and PERFECT move the ef_search dial, and
    // SearchQuality.Custom(ef) sets it outright.
    println("\nvector search, accurate profile:")
    notes.searchWithQuality(query, 3u, SearchQuality.ACCURATE).forEach { hit ->
        println("  #${hit.id}  score ${hit.score}")
    }

    // ---- 5. Filtered search -----------------------------------------------
    // The filter is a JSON string with the same grammar as the REST server:
    //   {"condition": {"type": "eq", "field": "...", "value": ...}}
    // Comparison types: eq, neq, gt, gte, lt, lte; "and" / "or" take a
    // `conditions` array and nest.
    val filterJson = """{"condition": {"type": "eq", "field": "category", "value": "shopping"}}"""
    println("\nvector search restricted to category = shopping:")
    notes.searchWithFilter(query, 3u, filterJson).forEach { hit ->
        println("  #${hit.id}  score ${hit.score}")
    }

    // ---- 6. Text and hybrid retrieval -------------------------------------
    // BM25 over the payload strings, no extra index to build.
    println("\nBM25 text search for \"coffee\":")
    notes.textSearch("coffee", 3u).forEach { hit ->
        println("  #${hit.id}  score ${hit.score}")
    }

    // Hybrid fuses both rankings. vectorWeight is the vector share:
    // 0.0 = text only, 1.0 = vector only.
    println("\nhybrid search (70% vector / 30% text):")
    notes.hybridSearch(query, "coffee", 3u, 0.7f).forEach { hit ->
        println("  #${hit.id}  score ${hit.score}")
    }

    // ---- 7. VelesQL -------------------------------------------------------
    // Rows come back as JSON object strings — UniFFI cannot carry a dynamic map
    // across the FFI boundary, so each row is parsed on the Kotlin side.
    val result = db.executeQuery("SELECT * FROM notes LIMIT 10", null)
    println("\nVelesQL: ${result.message} (${result.rowCount} rows)")
    result.rows.forEach { row ->
        println("  ${JSONObject(row.dataJson)}")
    }

    // ---- 8. Knowledge graph ------------------------------------------------
    // MobileGraphStore is a deliberate in-memory fork of core's graph engine:
    // RAM-only, no WAL, no on-disk payloads — hence the explicit save/load.
    val graph = MobileGraphStore()
    graph.addNode(MobileGraphNode(
        id = 1uL, label = "Person", propertiesJson = """{"name":"Ada"}""", vector = null,
    ))
    graph.addNode(MobileGraphNode(
        id = 2uL, label = "Person", propertiesJson = """{"name":"Grace"}""", vector = null,
    ))
    graph.addNode(MobileGraphNode(
        id = 3uL, label = "Project", propertiesJson = """{"name":"Sync"}""", vector = null,
    ))
    // addEdge throws when an endpoint has no stored node.
    graph.addEdge(MobileGraphEdge(
        id = 100uL, source = 1uL, target = 2uL, label = "KNOWS", propertiesJson = null,
    ))
    graph.addEdge(MobileGraphEdge(
        id = 101uL, source = 2uL, target = 3uL, label = "WORKS_ON", propertiesJson = null,
    ))

    println("\ngraph: ${graph.nodeCount()} nodes, ${graph.edgeCount()} edges")
    println("BFS from node 1, depth <= 2:")
    graph.bfsTraverse(1uL, 2u, 100u).forEach { step ->
        println("  node ${step.nodeId} at depth ${step.depth} via ${step.path}")
    }
    // The source itself is never emitted (depth 0 is skipped) and every node is
    // visited at most once, so a diamond does not produce duplicates.

    // Persist it explicitly — nothing about this store is automatic.
    val graphPath = File(path, "graph.bin").absolutePath
    graph.save(graphPath)
    val reloaded = MobileGraphStore.load(graphPath)
    println("reloaded graph: ${reloaded.nodeCount()} nodes")

    // ---- 9. Agent memory ---------------------------------------------------
    // VelesSemanticMemory keeps facts with their embeddings and reads the text
    // back out of the payload, so content survives a database reload.
    // Semantic only: episodic and procedural memory are not exposed on mobile.
    val memory = VelesSemanticMemory(db, 4u)
    memory.store(1uL, "The user prefers dark roast coffee.",
        fakeEmbedding("The user prefers dark roast coffee."))
    memory.store(2uL, "The user is allergic to penicillin.",
        fakeEmbedding("The user is allergic to penicillin."))

    println("\nsemantic memory (${memory.len()} facts):")
    memory.query(fakeEmbedding("coffee preference"), 2u).forEach { hit ->
        println("  #${hit.id}  score ${hit.score}  ${hit.content}")
    }

    // ---- 10. The errors you will actually hit ------------------------------
    // A vector whose length differs from the collection's dimension.
    try {
        notes.search(listOf(1.0f, 0.0f), 1u)
        println("\nUNEXPECTED: a 2-dimensional query should not have been accepted")
    } catch (e: Exception) {
        // The generated exception hierarchy is VelesException, with Database,
        // Collection and DimensionMismatch subclasses. Match on the generated
        // names rather than guessing here; `e` is always printable.
        println("\ndimension mismatch rejected as expected: $e")
    }

    // A payload that is not JSON. Nothing is committed when this fires.
    try {
        notes.upsert(VelesPoint(id = 99uL, vector = fakeEmbedding("bad"), payload = "not json"))
        println("UNEXPECTED: a non-JSON payload should not have been accepted")
    } catch (e: Exception) {
        println("invalid payload rejected as expected: $e")
    }

    // streamInsert before enableStreaming.
    try {
        notes.streamInsert(listOf(
            VelesPoint(id = 98uL, vector = fakeEmbedding("stream"), payload = null),
        ))
        println("UNEXPECTED: streaming was never enabled")
    } catch (e: Exception) {
        println("stream insert without enableStreaming rejected as expected: $e")
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/**
 * Call this from an Activity, a ViewModel, or an instrumented test.
 *
 * It hops onto Dispatchers.IO because every binding call is blocking and a
 * search on the main thread freezes the UI.
 *
 * @param scope a coroutine scope, e.g. `viewModelScope` or `lifecycleScope`
 * @param filesDir the app's private directory, e.g. `context.filesDir`
 */
fun runQuickstartAsync(scope: CoroutineScope, filesDir: File) {
    scope.launch(Dispatchers.IO) {
        val dir = File(filesDir, "velesdb").apply { mkdirs() }
        try {
            runQuickstart(dir.absolutePath)
        } catch (e: Exception) {
            println("velesdb quickstart failed: $e")
        }
    }
}
