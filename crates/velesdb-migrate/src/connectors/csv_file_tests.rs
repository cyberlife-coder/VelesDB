//! Tests for CSV file connector.

use super::*;
use std::io::Write;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_csv_connector_json_vector() {
    let csv_content = "id,vector,title\n1,[0.1, 0.2, 0.3],Doc 1\n2,[0.4, 0.5, 0.6],Doc 2";

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(csv_content.as_bytes()).unwrap();

    let config = CsvFileConfig {
        path: file.path().to_path_buf(),
        id_column: "id".to_string(),
        vector_column: "vector".to_string(),
        vector_spread: false,
        dim_prefix: "dim_".to_string(),
        delimiter: ',',
        has_header: true,
    };

    let mut connector = CsvFileConnector::new(config);
    connector.connect().await.unwrap();
    let schema = connector.get_schema().await.unwrap();
    let batch = connector.extract_batch(None, 10).await.unwrap();

    assert_eq!(schema.dimension, 3);
    assert_eq!(batch.points.len(), 2);
    assert_eq!(batch.points[0].id, "1");
    assert_eq!(batch.points[0].vector, vec![0.1, 0.2, 0.3]);
}

#[tokio::test]
async fn test_csv_connector_spread_vector() {
    let csv_content = "id,dim_0,dim_1,dim_2,title\na,0.1,0.2,0.3,Test\nb,0.4,0.5,0.6,Test2";

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(csv_content.as_bytes()).unwrap();

    let config = CsvFileConfig {
        path: file.path().to_path_buf(),
        id_column: "id".to_string(),
        vector_column: "vector".to_string(),
        vector_spread: true,
        dim_prefix: "dim_".to_string(),
        delimiter: ',',
        has_header: true,
    };

    let mut connector = CsvFileConnector::new(config);
    connector.connect().await.unwrap();
    let batch = connector.extract_batch(None, 10).await.unwrap();

    assert_eq!(batch.points[0].vector, vec![0.1, 0.2, 0.3]);
    assert_eq!(batch.points[1].vector, vec![0.4, 0.5, 0.6]);
}

#[tokio::test]
async fn test_csv_connector_tab_delimiter() {
    let csv_content = "id\tvector\ttitle\n1\t[1.0, 2.0]\tDoc";

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(csv_content.as_bytes()).unwrap();

    let config = CsvFileConfig {
        path: file.path().to_path_buf(),
        id_column: "id".to_string(),
        vector_column: "vector".to_string(),
        vector_spread: false,
        dim_prefix: "dim_".to_string(),
        delimiter: '\t',
        has_header: true,
    };

    let mut connector = CsvFileConnector::new(config);
    connector.connect().await.unwrap();
    let batch = connector.extract_batch(None, 10).await.unwrap();

    assert_eq!(batch.points[0].id, "1");
    assert_eq!(batch.points[0].vector, vec![1.0, 2.0]);
}

#[tokio::test]
async fn test_csv_connector_no_header() {
    let csv_content = "1,[0.1,0.2],test\n2,[0.3,0.4],test2";

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(csv_content.as_bytes()).unwrap();

    let config = CsvFileConfig {
        path: file.path().to_path_buf(),
        id_column: "col_0".to_string(),
        vector_column: "col_1".to_string(),
        vector_spread: false,
        dim_prefix: "dim_".to_string(),
        delimiter: ',',
        has_header: false,
    };

    let mut connector = CsvFileConnector::new(config);
    connector.connect().await.unwrap();
    let batch = connector.extract_batch(None, 10).await.unwrap();

    assert_eq!(batch.points.len(), 2);
    assert_eq!(batch.points[0].id, "1");
}

#[tokio::test]
async fn test_csv_connector_missing_vector_column() {
    let csv_content = "id,title\n1,Test";

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(csv_content.as_bytes()).unwrap();

    let config = CsvFileConfig {
        path: file.path().to_path_buf(),
        id_column: "id".to_string(),
        vector_column: "vector".to_string(),
        vector_spread: false,
        dim_prefix: "dim_".to_string(),
        delimiter: ',',
        has_header: true,
    };

    let mut connector = CsvFileConnector::new(config);
    let result = connector.connect().await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}
