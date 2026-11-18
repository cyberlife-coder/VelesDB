//! Point ids cross the IPC boundary as REST carries them: written as decimal
//! strings, read as a decimal string or a number. A JavaScript number rounds
//! any id past 2^53, so a numeric id handed back to JS was silently another
//! point's id.

use serde_json::{json, Value};

use super::{
    DeletePointsRequest, EpisodicDeleteRequest, EpisodicRecordRequest, EpisodicResult,
    EpisodicSimilarResult, GetPointsRequest, HybridResult, IdScoreOutput, MemoryTtlRequest,
    MetadataPointInput, PointInput, PointOutput, ProceduralDeleteRequest, ProceduralLearnRequest,
    ProceduralMatchResult, ProceduralReinforceRequest, ScrollRequest, ScrollResponse,
    SemanticDeleteRequest, SemanticQueryResult, SemanticStoreRequest, SemanticStoreWithTtlRequest,
    SparsePointInput,
};

/// 2^53 + 1: the first integer a JavaScript number cannot hold.
const BIG: u64 = (1_u64 << 53) + 1;
const BIG_STR: &str = "9007199254740993";

fn to_json(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("test: serialize")
}

#[test]
fn every_output_writes_its_point_id_as_a_decimal_string() {
    let hits = to_json(&IdScoreOutput {
        id: BIG,
        score: 0.5,
    });
    assert_eq!(hits["id"], BIG_STR, "search_ids");
    let point = to_json(&PointOutput {
        id: BIG,
        vector: vec![],
        payload: None,
    });
    assert_eq!(point["id"], BIG_STR, "get / scroll points");
    let hybrid = to_json(&HybridResult {
        node_id: BIG,
        vector_score: None,
        graph_score: None,
        fused_score: 1.0,
        bindings: None,
        column_data: None,
    });
    assert_eq!(hybrid["nodeId"], BIG_STR, "query");
    let semantic = to_json(&SemanticQueryResult {
        id: BIG,
        score: 0.5,
        content: String::new(),
    });
    assert_eq!(semantic["id"], BIG_STR, "semantic_query");
    let episode = to_json(&EpisodicResult {
        id: BIG,
        content: String::new(),
        timestamp: 0,
    });
    assert_eq!(episode["id"], BIG_STR, "episodic_recent");
    let similar = to_json(&EpisodicSimilarResult {
        id: BIG,
        content: String::new(),
        timestamp: 0,
        score: 0.5,
    });
    assert_eq!(similar["id"], BIG_STR, "episodic_recall_similar");
    let procedure = to_json(&ProceduralMatchResult {
        id: BIG,
        name: String::new(),
        steps: vec![],
        confidence: 1.0,
        score: 0.5,
    });
    assert_eq!(procedure["id"], BIG_STR, "procedural_recall");
}

#[test]
fn a_scroll_cursor_round_trips_as_a_string() {
    let page = to_json(&ScrollResponse {
        points: vec![],
        next_cursor: Some(BIG),
    });
    assert_eq!(page["nextCursor"], BIG_STR, "as REST's ScrollResponse");
    let last = to_json(&ScrollResponse {
        points: vec![],
        next_cursor: None,
    });
    assert!(last.get("nextCursor").is_none(), "absent at the end");

    for cursor in [json!(BIG_STR), json!(7)] {
        let request: ScrollRequest =
            serde_json::from_value(json!({"collection": "c", "cursor": cursor}))
                .expect("test: a string or a number cursor");
        assert!(request.cursor.is_some());
    }
    let request: ScrollRequest =
        serde_json::from_value(json!({"collection": "c", "cursor": BIG_STR})).expect("test");
    assert_eq!(request.cursor, Some(BIG));
    let first: ScrollRequest =
        serde_json::from_value(json!({"collection": "c"})).expect("test: no cursor");
    assert_eq!(first.cursor, None);
}

fn read<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("test: deserialize")
}

#[test]
fn every_input_reads_a_point_id_as_a_string_or_a_number() {
    for id in [json!(BIG_STR), json!(BIG)] {
        let point: PointInput = read(json!({"id": id, "vector": [1.0], "payload": null}));
        assert_eq!(point.id, BIG);
        let metadata: MetadataPointInput = read(json!({"id": id, "payload": {}}));
        assert_eq!(metadata.id, BIG);
        let sparse: SparsePointInput = read(json!({"id": id, "vector": [1.0], "payload": null}));
        assert_eq!(sparse.id, BIG);
        let get: GetPointsRequest = read(json!({"collection": "c", "ids": [id, 2]}));
        assert_eq!(get.ids, vec![BIG, 2]);
        let delete: DeletePointsRequest = read(json!({"collection": "c", "ids": [id]}));
        assert_eq!(delete.ids, vec![BIG]);
        let ss: SemanticStoreRequest = read(json!({"id": id, "content": "", "embedding": []}));
        assert_eq!(ss.id, BIG);
        let st: SemanticStoreWithTtlRequest =
            read(json!({"id": id, "content": "", "embedding": [], "ttlSeconds": 5}));
        assert_eq!(st.id, BIG);
        let sd: SemanticDeleteRequest = read(json!({"id": id}));
        assert_eq!(sd.id, BIG);
        let er: EpisodicRecordRequest =
            read(json!({"eventId": id, "content": "", "timestamp": 0, "embedding": []}));
        assert_eq!(er.event_id, BIG);
        let ed: EpisodicDeleteRequest = read(json!({"eventId": id}));
        assert_eq!(ed.event_id, BIG);
        let pl: ProceduralLearnRequest =
            read(json!({"procedureId": id, "name": "", "steps": [], "embedding": []}));
        assert_eq!(pl.procedure_id, BIG);
        let pr: ProceduralReinforceRequest = read(json!({"procedureId": id, "success": true}));
        assert_eq!(pr.procedure_id, BIG);
        let pd: ProceduralDeleteRequest = read(json!({"procedureId": id}));
        assert_eq!(pd.procedure_id, BIG);
        let ttl: MemoryTtlRequest = read(json!({"kind": "semantic", "id": id, "ttlSeconds": 5}));
        assert_eq!(ttl.id, BIG);
    }
}
