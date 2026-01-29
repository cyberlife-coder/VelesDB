//! Tests for RAG commands module

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    /// Simple Chunk struct for tests
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct Chunk {
        id: u64,
        text: String,
        score: Option<f32>,
    }

    /// Simple SearchResult struct for tests
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct SearchResult {
        chunks: Vec<Chunk>,
        query: String,
        time_ms: f64,
    }

    /// Simple IndexStats struct for tests
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct IndexStats {
        total_chunks: usize,
        dimension: usize,
    }

    /// Test chunk text function
    fn chunk_text(text: &str, chunk_size: usize) -> Vec<String> {
        let paragraphs: Vec<&str> = text.split("\n\n").collect();
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();

        for para in paragraphs {
            if current_chunk.len() + para.len() > chunk_size && !current_chunk.is_empty() {
                chunks.push(current_chunk.trim().to_string());
                current_chunk = String::new();
            }
            current_chunk.push_str(para);
            current_chunk.push_str("\n\n");
        }

        if !current_chunk.trim().is_empty() {
            chunks.push(current_chunk.trim().to_string());
        }

        chunks
    }

    #[test]
    fn test_chunk_text_single_paragraph() {
        let text = "This is a short paragraph.";
        let chunks = chunk_text(text, 500);
        
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("short paragraph"));
    }

    #[test]
    fn test_chunk_text_multiple_paragraphs() {
        let text = "First paragraph here.\n\nSecond paragraph here.\n\nThird paragraph.";
        let chunks = chunk_text(text, 500);
        
        assert_eq!(chunks.len(), 1); // All fit in one chunk
        assert!(chunks[0].contains("First"));
        assert!(chunks[0].contains("Third"));
    }

    #[test]
    fn test_chunk_text_split_large() {
        let text = "First long paragraph with lots of content here.\n\nSecond paragraph.";
        let chunks = chunk_text(text, 30); // Small chunk size
        
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn test_chunk_text_empty() {
        let text = "";
        let chunks = chunk_text(text, 500);
        
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_serialization() {
        let chunk = Chunk {
            id: 42,
            text: "Test chunk content".to_string(),
            score: Some(0.95),
        };
        
        let json = serde_json::to_string(&chunk).unwrap();
        let deserialized: Chunk = serde_json::from_str(&json).unwrap();
        
        assert_eq!(deserialized.id, 42);
        assert_eq!(deserialized.text, "Test chunk content");
        assert_eq!(deserialized.score, Some(0.95));
    }

    #[test]
    fn test_search_result_serialization() {
        let result = SearchResult {
            chunks: vec![
                Chunk { id: 1, text: "First".to_string(), score: Some(0.9) },
                Chunk { id: 2, text: "Second".to_string(), score: Some(0.8) },
            ],
            query: "test query".to_string(),
            time_ms: 5.5,
        };
        
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: SearchResult = serde_json::from_str(&json).unwrap();
        
        assert_eq!(deserialized.chunks.len(), 2);
        assert_eq!(deserialized.query, "test query");
        assert!(deserialized.time_ms > 5.0);
    }

    #[test]
    fn test_index_stats_serialization() {
        let stats = IndexStats {
            total_chunks: 100,
            dimension: 384,
        };
        
        let json = serde_json::to_string(&stats).unwrap();
        let deserialized: IndexStats = serde_json::from_str(&json).unwrap();
        
        assert_eq!(deserialized.total_chunks, 100);
        assert_eq!(deserialized.dimension, 384);
    }

    #[test]
    fn test_chunk_store_map_operations() {
        let mut store: HashMap<u64, String> = HashMap::new();
        
        store.insert(1, "Hello world".to_string());
        store.insert(2, "Test chunk".to_string());
        
        assert_eq!(store.len(), 2);
        assert_eq!(store.get(&1), Some(&"Hello world".to_string()));
        assert_eq!(store.get(&3), None);
    }
}
